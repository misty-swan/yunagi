-- マイグレーション 3：障害の解消メモも全文検索の対象にする（docs/design/02-data-model.md 4章・4.1）。
-- 一度配布したら書き換えない。変更は新しい番号のマイグレーションで足す。
--
-- - search_index に列 resolution_note を足す（FTS5 の仮想テーブルは列を足せないので作り直す）。
--   本文（body）とは別の列にするのは、本文の末尾と解消メモの先頭にまたがる語に一致させないため。
--   FTS5 の句は列をまたがない。
-- - 索引に入れるのは、解消済み（status = 'resolved'）の障害の解消メモだけ。未解消に戻した障害に
--   残っている解消メモは画面に出ない（画面設計書 3章）ので、検索でも出さない。
-- - search_index_ref はそのまま使う（id は search_index の rowid なので、同じ id で入れ直す）。
-- - note・reflection のトリガーは列を指定して入れるので（resolution_note は NULL になる）、そのまま使う。
--   item の追加・更新のトリガーだけを作り直す（削除のトリガーは行番号で消すだけなので、そのまま）。

DROP TABLE search_index;

CREATE VIRTUAL TABLE search_index USING fts5 (
    body,
    resolution_note,
    content = '',
    contentless_delete = 1,
    tokenize = 'trigram'
);

DROP TRIGGER search_item_insert;
DROP TRIGGER search_item_update;

CREATE TRIGGER search_item_insert AFTER INSERT ON item
WHEN new.deleted_at IS NULL
BEGIN
    INSERT INTO search_index_ref (source, ref_id, kind, date)
        VALUES ('item', new.id, new.kind, new.date);
    INSERT INTO search_index (rowid, body, resolution_note)
        VALUES (
            last_insert_rowid(),
            new.text,
            CASE WHEN new.status = 'resolved' THEN new.resolution_note END
        );
END;

-- 本文・種類・日付・削除の有無に加え、索引に入れる解消メモ（解消済みかどうかと、解消メモ）が
-- 変わったときに入れ直す
CREATE TRIGGER search_item_update AFTER UPDATE ON item
WHEN old.text IS NOT new.text
    OR old.kind IS NOT new.kind
    OR old.date IS NOT new.date
    OR old.id IS NOT new.id
    OR (old.deleted_at IS NULL) <> (new.deleted_at IS NULL)
    OR (CASE WHEN old.status = 'resolved' THEN old.resolution_note END)
        IS NOT (CASE WHEN new.status = 'resolved' THEN new.resolution_note END)
BEGIN
    DELETE FROM search_index WHERE rowid =
        (SELECT id FROM search_index_ref WHERE source = 'item' AND ref_id = old.id);
    DELETE FROM search_index_ref WHERE source = 'item' AND ref_id = old.id;
    INSERT INTO search_index_ref (source, ref_id, kind, date)
        SELECT 'item', new.id, new.kind, new.date WHERE new.deleted_at IS NULL;
    INSERT INTO search_index (rowid, body, resolution_note)
        SELECT
            last_insert_rowid(),
            new.text,
            CASE WHEN new.status = 'resolved' THEN new.resolution_note END
        WHERE new.deleted_at IS NULL;
END;

-- 作り直した索引に、今ある行を入れ直す
INSERT INTO search_index (rowid, body, resolution_note)
    SELECT r.id, i.text, CASE WHEN i.status = 'resolved' THEN i.resolution_note END
    FROM search_index_ref AS r JOIN item AS i ON r.source = 'item' AND r.ref_id = i.id;
INSERT INTO search_index (rowid, body)
    SELECT r.id, n.body FROM search_index_ref AS r JOIN note AS n ON r.source = 'note' AND r.ref_id = n.date;
INSERT INTO search_index (rowid, body)
    SELECT r.id, d.body FROM search_index_ref AS r
    JOIN reflection AS d ON r.source = 'reflection' AND r.ref_id = d.date;
