-- マイグレーション 2：全文検索の索引（docs/design/02-data-model.md 4章）。
-- 一度配布したら書き換えない。変更は新しい番号のマイグレーションで足す。
--
-- - search_index は FTS5 の仮想テーブルで、trigram トークナイザ（3文字ずつの並び）で body を索引する。
--   日本語は単語の区切りが無いため。大文字・小文字は区別しない（trigram の既定）。
--   kind・ref_id・date は索引せず、結果と絞り込みに使う。
--   - kind：`todo` / `blocker`（item.kind）、`note`、`reflection`
--   - ref_id：item は item.id、note・reflection は date
-- - search_index_ref は、元の行（source と ref_id）から search_index の行番号（rowid）を引く表。
--   FTS5 は索引しない列（ref_id）で行を探すと全件を読む（ノートの本文も含む）ため、
--   元の行の更新のたびに全件を読まずに済むよう、行番号で消す。
-- - item・note・reflection の追加・更新・削除のたびに、トリガーで索引をそろえる。
--   論理削除した項目（deleted_at が NULL でないもの）は索引に入れない。
--   更新は、本文・種類・日付・削除の有無のどれかが変わったときだけ入れ直す（状態の変更などでは触らない）。

CREATE VIRTUAL TABLE search_index USING fts5 (
    kind UNINDEXED,
    ref_id UNINDEXED,
    date UNINDEXED,
    body,
    tokenize = 'trigram'
);

CREATE TABLE search_index_ref (
    id     INTEGER PRIMARY KEY, -- search_index の rowid
    source TEXT NOT NULL,       -- 元の表：`item` / `note` / `reflection`
    ref_id TEXT NOT NULL,
    UNIQUE (source, ref_id)
) STRICT;

-- item

CREATE TRIGGER search_item_insert AFTER INSERT ON item
WHEN new.deleted_at IS NULL
BEGIN
    INSERT INTO search_index_ref (source, ref_id) VALUES ('item', new.id);
    INSERT INTO search_index (rowid, kind, ref_id, date, body)
        VALUES (last_insert_rowid(), new.kind, new.id, new.date, new.text);
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
    INSERT INTO search_index_ref (source, ref_id)
        SELECT 'item', new.id WHERE new.deleted_at IS NULL;
    INSERT INTO search_index (rowid, kind, ref_id, date, body)
        SELECT last_insert_rowid(), new.kind, new.id, new.date, new.text
        WHERE new.deleted_at IS NULL;
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
    INSERT INTO search_index_ref (source, ref_id) VALUES ('note', new.date);
    INSERT INTO search_index (rowid, kind, ref_id, date, body)
        VALUES (last_insert_rowid(), 'note', new.date, new.date, new.body);
END;

CREATE TRIGGER search_note_update AFTER UPDATE ON note
WHEN old.body IS NOT new.body OR old.date IS NOT new.date
BEGIN
    DELETE FROM search_index WHERE rowid =
        (SELECT id FROM search_index_ref WHERE source = 'note' AND ref_id = old.date);
    DELETE FROM search_index_ref WHERE source = 'note' AND ref_id = old.date;
    INSERT INTO search_index_ref (source, ref_id) VALUES ('note', new.date);
    INSERT INTO search_index (rowid, kind, ref_id, date, body)
        VALUES (last_insert_rowid(), 'note', new.date, new.date, new.body);
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
    INSERT INTO search_index_ref (source, ref_id) VALUES ('reflection', new.date);
    INSERT INTO search_index (rowid, kind, ref_id, date, body)
        VALUES (last_insert_rowid(), 'reflection', new.date, new.date, new.body);
END;

CREATE TRIGGER search_reflection_update AFTER UPDATE ON reflection
WHEN old.body IS NOT new.body OR old.date IS NOT new.date
BEGIN
    DELETE FROM search_index WHERE rowid =
        (SELECT id FROM search_index_ref WHERE source = 'reflection' AND ref_id = old.date);
    DELETE FROM search_index_ref WHERE source = 'reflection' AND ref_id = old.date;
    INSERT INTO search_index_ref (source, ref_id) VALUES ('reflection', new.date);
    INSERT INTO search_index (rowid, kind, ref_id, date, body)
        VALUES (last_insert_rowid(), 'reflection', new.date, new.date, new.body);
END;

CREATE TRIGGER search_reflection_delete AFTER DELETE ON reflection
BEGIN
    DELETE FROM search_index WHERE rowid =
        (SELECT id FROM search_index_ref WHERE source = 'reflection' AND ref_id = old.date);
    DELETE FROM search_index_ref WHERE source = 'reflection' AND ref_id = old.date;
END;

-- マイグレーション 1 の時点ですでにある行を索引に入れる
INSERT INTO search_index_ref (source, ref_id)
    SELECT 'item', id FROM item WHERE deleted_at IS NULL;
INSERT INTO search_index_ref (source, ref_id) SELECT 'note', date FROM note;
INSERT INTO search_index_ref (source, ref_id) SELECT 'reflection', date FROM reflection;
INSERT INTO search_index (rowid, kind, ref_id, date, body)
    SELECT r.id, i.kind, i.id, i.date, i.text
    FROM search_index_ref AS r JOIN item AS i ON r.source = 'item' AND r.ref_id = i.id;
INSERT INTO search_index (rowid, kind, ref_id, date, body)
    SELECT r.id, 'note', n.date, n.date, n.body
    FROM search_index_ref AS r JOIN note AS n ON r.source = 'note' AND r.ref_id = n.date;
INSERT INTO search_index (rowid, kind, ref_id, date, body)
    SELECT r.id, 'reflection', d.date, d.date, d.body
    FROM search_index_ref AS r JOIN reflection AS d ON r.source = 'reflection' AND r.ref_id = d.date;
