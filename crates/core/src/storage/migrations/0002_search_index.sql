-- マイグレーション 2：全文検索の索引（docs/design/02-data-model.md 4章）。
-- 一度配布したら書き換えない。変更は新しい番号のマイグレーションで足す。
--
-- - search_index は FTS5 の仮想テーブルで、trigram トークナイザ（3文字ずつの並び）で body を索引する。
--   日本語は単語の区切りが無いため。大文字・小文字は区別しない（trigram の既定）。
--   本文の写しは持たない（content=''）。本文は元の表から読む。行は rowid で消す（contentless_delete）。
-- - search_index_ref は、search_index の各行（rowid = id）が元のどの行かと、結果と絞り込みに使う
--   種類・日付を持つ。並べ替えと件数の切り取りはこの表で行い、本文を読まずに済ませる。
--   - source：元の表（`item` / `note` / `reflection`）
--   - ref_id：item は item.id、note・reflection は date
--   - kind：`todo` / `blocker`（item.kind）、`note`、`reflection`
-- - item・note・reflection の追加・更新・削除のたびに、トリガーで索引をそろえる。
--   論理削除した項目（deleted_at が NULL でないもの）は索引に入れない。
--   更新は、本文・種類・日付・削除の有無のどれかが変わったときだけ入れ直す（状態の変更などでは触らない）。
-- - 後のマイグレーションで item・note・reflection を作り直す（DROP TABLE する）と、ここのトリガーも
--   消える。作り直すときは、同じマイグレーションでトリガーも作り直す（設計書 4.1）。

CREATE VIRTUAL TABLE search_index USING fts5 (
    body,
    content = '',
    contentless_delete = 1,
    tokenize = 'trigram'
);

CREATE TABLE search_index_ref (
    id     INTEGER PRIMARY KEY, -- search_index の rowid
    source TEXT NOT NULL,
    ref_id TEXT NOT NULL,
    kind   TEXT NOT NULL,
    date   TEXT NOT NULL,
    UNIQUE (source, ref_id)
) STRICT;

-- 期間で絞って新しい順に並べる
CREATE INDEX search_index_ref_date ON search_index_ref (date);

-- item

CREATE TRIGGER search_item_insert AFTER INSERT ON item
WHEN new.deleted_at IS NULL
BEGIN
    INSERT INTO search_index_ref (source, ref_id, kind, date)
        VALUES ('item', new.id, new.kind, new.date);
    INSERT INTO search_index (rowid, body) VALUES (last_insert_rowid(), new.text);
END;

CREATE TRIGGER search_item_update AFTER UPDATE ON item
WHEN old.text IS NOT new.text
    OR old.kind IS NOT new.kind
    OR old.date IS NOT new.date
    OR old.id IS NOT new.id
    OR (old.deleted_at IS NULL) <> (new.deleted_at IS NULL)
BEGIN
    DELETE FROM search_index WHERE rowid =
        (SELECT id FROM search_index_ref WHERE source = 'item' AND ref_id = old.id);
    DELETE FROM search_index_ref WHERE source = 'item' AND ref_id = old.id;
    INSERT INTO search_index_ref (source, ref_id, kind, date)
        SELECT 'item', new.id, new.kind, new.date WHERE new.deleted_at IS NULL;
    INSERT INTO search_index (rowid, body)
        SELECT last_insert_rowid(), new.text WHERE new.deleted_at IS NULL;
END;

CREATE TRIGGER search_item_delete AFTER DELETE ON item
BEGIN
    DELETE FROM search_index WHERE rowid =
        (SELECT id FROM search_index_ref WHERE source = 'item' AND ref_id = old.id);
    DELETE FROM search_index_ref WHERE source = 'item' AND ref_id = old.id;
END;

-- note

CREATE TRIGGER search_note_insert AFTER INSERT ON note
BEGIN
    INSERT INTO search_index_ref (source, ref_id, kind, date)
        VALUES ('note', new.date, 'note', new.date);
    INSERT INTO search_index (rowid, body) VALUES (last_insert_rowid(), new.body);
END;

CREATE TRIGGER search_note_update AFTER UPDATE ON note
WHEN old.body IS NOT new.body OR old.date IS NOT new.date
BEGIN
    DELETE FROM search_index WHERE rowid =
        (SELECT id FROM search_index_ref WHERE source = 'note' AND ref_id = old.date);
    DELETE FROM search_index_ref WHERE source = 'note' AND ref_id = old.date;
    INSERT INTO search_index_ref (source, ref_id, kind, date)
        SELECT 'note', new.date, 'note', new.date;
    INSERT INTO search_index (rowid, body)
        SELECT last_insert_rowid(), new.body;
END;

CREATE TRIGGER search_note_delete AFTER DELETE ON note
BEGIN
    DELETE FROM search_index WHERE rowid =
        (SELECT id FROM search_index_ref WHERE source = 'note' AND ref_id = old.date);
    DELETE FROM search_index_ref WHERE source = 'note' AND ref_id = old.date;
END;

-- reflection

CREATE TRIGGER search_reflection_insert AFTER INSERT ON reflection
BEGIN
    INSERT INTO search_index_ref (source, ref_id, kind, date)
        VALUES ('reflection', new.date, 'reflection', new.date);
    INSERT INTO search_index (rowid, body) VALUES (last_insert_rowid(), new.body);
END;

CREATE TRIGGER search_reflection_update AFTER UPDATE ON reflection
WHEN old.body IS NOT new.body OR old.date IS NOT new.date
BEGIN
    DELETE FROM search_index WHERE rowid =
        (SELECT id FROM search_index_ref WHERE source = 'reflection' AND ref_id = old.date);
    DELETE FROM search_index_ref WHERE source = 'reflection' AND ref_id = old.date;
    INSERT INTO search_index_ref (source, ref_id, kind, date)
        SELECT 'reflection', new.date, 'reflection', new.date;
    INSERT INTO search_index (rowid, body)
        SELECT last_insert_rowid(), new.body;
END;

CREATE TRIGGER search_reflection_delete AFTER DELETE ON reflection
BEGIN
    DELETE FROM search_index WHERE rowid =
        (SELECT id FROM search_index_ref WHERE source = 'reflection' AND ref_id = old.date);
    DELETE FROM search_index_ref WHERE source = 'reflection' AND ref_id = old.date;
END;

-- マイグレーション 1 の時点ですでにある行を索引に入れる
INSERT INTO search_index_ref (source, ref_id, kind, date)
    SELECT 'item', id, kind, date FROM item WHERE deleted_at IS NULL;
INSERT INTO search_index_ref (source, ref_id, kind, date) SELECT 'note', date, 'note', date FROM note;
INSERT INTO search_index_ref (source, ref_id, kind, date)
    SELECT 'reflection', date, 'reflection', date FROM reflection;
INSERT INTO search_index (rowid, body)
    SELECT r.id, i.text FROM search_index_ref AS r JOIN item AS i ON r.source = 'item' AND r.ref_id = i.id;
INSERT INTO search_index (rowid, body)
    SELECT r.id, n.body FROM search_index_ref AS r JOIN note AS n ON r.source = 'note' AND r.ref_id = n.date;
INSERT INTO search_index (rowid, body)
    SELECT r.id, d.body FROM search_index_ref AS r
    JOIN reflection AS d ON r.source = 'reflection' AND r.ref_id = d.date;
