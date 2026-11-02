-- マイグレーション 1：最初の表（docs/design/02-data-model.md 3章）。
-- 一度配布したら書き換えない。変更は新しい番号のマイグレーションで足す。
--
-- - 時刻はすべて UTC の ISO 8601（`YYYY-MM-DDTHH:MM:SS.sssZ`。設計書 1章）。
-- - 日付は論理日付 `YYYY-MM-DD`。
-- - id は ULID（26 文字）。
-- - 状態などの値（kind・status など）は、検査（CHECK）を付けずにコアの型で確かめる。
--   値を足すたびに表を作り直さずに済むようにするため。ただし値を足すときは、旧版が
--   知らない値を読んで止まらないよう、新しい番号のマイグレーションを足す（設計書 1.1）。
-- - JSON の列は、正しい JSON であること（scopes は配列であること）だけを確かめる。
-- - external_ref は MVP 後の機能のため、まだ作らない（item.external_ref_id は参照先を持たない列）。

CREATE TABLE day (
    date                   TEXT NOT NULL PRIMARY KEY,
    tz                     TEXT NOT NULL,
    carryover_done_at      TEXT,
    carryover_postponed_at TEXT,
    carryover_reshown_at   TEXT,
    created_at             TEXT NOT NULL
) STRICT;

CREATE TABLE agent_client (
    id           TEXT NOT NULL PRIMARY KEY,
    name         TEXT NOT NULL UNIQUE,
    scopes       TEXT NOT NULL CHECK (json_valid(scopes) AND json_type(scopes) = 'array'),
    created_at   TEXT NOT NULL,
    last_seen_at TEXT
) STRICT;

CREATE TABLE item (
    id              TEXT NOT NULL PRIMARY KEY,
    date            TEXT NOT NULL REFERENCES day (date),
    kind            TEXT NOT NULL,
    text            TEXT NOT NULL,
    status          TEXT NOT NULL,
    close_reason    TEXT,
    sort_key        TEXT NOT NULL,
    carry_count     INTEGER NOT NULL,
    carried_from_id TEXT REFERENCES item (id),
    split_from_id   TEXT REFERENCES item (id),
    resolution_note TEXT,
    origin          TEXT NOT NULL,
    agent_client_id TEXT REFERENCES agent_client (id),
    external_ref_id TEXT,
    completed_at    TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    deleted_at      TEXT
) STRICT;

-- その日のリストを並び順に出す
CREATE INDEX item_date_sort_key ON item (date, sort_key);
-- 持ち越し済み・分割済みかの判定（元を指す項目があるか）と、ノートの行へのさかのぼり
CREATE INDEX item_carried_from_id ON item (carried_from_id);
CREATE INDEX item_split_from_id ON item (split_from_id);
-- 外部キーの参照元の列。参照先の行を消す・id を変えるときに、表全体を調べずに済むように
CREATE INDEX item_agent_client_id ON item (agent_client_id) WHERE agent_client_id IS NOT NULL;

CREATE TABLE note (
    date       TEXT NOT NULL PRIMARY KEY REFERENCES day (date),
    body       TEXT NOT NULL,
    rev        INTEGER NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE reflection (
    date       TEXT NOT NULL PRIMARY KEY REFERENCES day (date),
    body       TEXT NOT NULL,
    rev        INTEGER NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE note_send (
    id               TEXT NOT NULL PRIMARY KEY,
    date             TEXT NOT NULL REFERENCES day (date),
    line_text        TEXT NOT NULL,
    line_fingerprint TEXT NOT NULL,
    item_id          TEXT NOT NULL REFERENCES item (id),
    created_at       TEXT NOT NULL
) STRICT;

-- 送信済みの印（その日のノートの行との照合）と、項目から元のノートの行へのさかのぼり
CREATE INDEX note_send_date ON note_send (date);
CREATE INDEX note_send_item_id ON note_send (item_id);

CREATE TABLE change_log (
    id              TEXT NOT NULL PRIMARY KEY,
    actor           TEXT NOT NULL,
    agent_client_id TEXT REFERENCES agent_client (id),
    operation       TEXT NOT NULL,
    target_type     TEXT NOT NULL,
    target_id       TEXT NOT NULL,
    before          TEXT CHECK (before IS NULL OR json_valid(before)),
    after           TEXT CHECK (after IS NULL OR json_valid(after)),
    created_at      TEXT NOT NULL,
    reverted_at     TEXT,
    revert_of_id    TEXT REFERENCES change_log (id),
    seen_at         TEXT
) STRICT;

-- 項目の行頭の菱形（その項目を指す記録があるか）と、取り消し前の確認
CREATE INDEX change_log_target ON change_log (target_type, target_id);
-- エージェントによる未確認の変更（上部のバーとサイドバーの件数）。確認すると索引から外れる
CREATE INDEX change_log_agent_client_id ON change_log (agent_client_id)
    WHERE agent_client_id IS NOT NULL;
CREATE INDEX change_log_revert_of_id ON change_log (revert_of_id) WHERE revert_of_id IS NOT NULL;
CREATE INDEX change_log_unseen_agent ON change_log (id) WHERE actor = 'agent' AND seen_at IS NULL;

CREATE TABLE setting (
    key   TEXT NOT NULL PRIMARY KEY,
    value TEXT NOT NULL CHECK (json_valid(value))
) STRICT;

-- 固定のクライアント（設計書 3章 agent_client）。id は固定の ULID（時刻の部分は 0）。
-- 既定のスコープは read のみ（MCP・CLI 仕様書 2章）。
INSERT INTO agent_client (id, name, scopes, created_at, last_seen_at) VALUES
    ('00000000000000000000000001', 'cli', '["read"]', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL),
    ('00000000000000000000000002', 'unknown', '["read"]', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL);
