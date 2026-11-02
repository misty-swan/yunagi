# データモデル設計書

## 1. 方針

- 1つの SQLite データベース（`yunagi.db`）に全データを保存する。
- 主キーは時刻順に並ぶ ULID とし、将来のクラウド同期で端末間の衝突を避ける。
- 削除は論理削除（`deleted_at`）とし、変更履歴からの取り消しと同期に備える。
- 日付は論理日付（切り替わり時刻を考慮した `YYYY-MM-DD`）で保持する。
- スキーマはマイグレーション番号で管理し、更新時に自動適用する。適用前に必ずバックアップを取る。適用に失敗した場合は自動でバックアップに戻してデータを変更せず、旧版の再導入方法を案内する（ビルド・配布手順書 5章）。
- `yunagi.db` は保存先（`data_dir`）に置く。保存先そのものは DB ではなく設定ファイル `config.json` に記録する（アーキテクチャ設計書 6章）。

## 2. エンティティ一覧

| エンティティ | 概要 |
| --- | --- |
| day | 論理日付ごとのボード。持ち越し処理の完了状態などを持つ |
| item | TODO と障害・詰まりを共通の構造で持つ |
| note | その日のノート本文（1日1枚） |
| reflection | その日の振り返り本文 |
| note_send | ノートの行を TODO・障害へ送った記録（送信済みの印に使う） |
| change_log | エージェント・CLI による変更の履歴と取り消し |
| agent_client | 接続したエージェントの識別と権限 |
| external_ref | 外部ツールの課題との紐付け（MVP 後の有料機能。ロードマップ参照） |
| setting | 設定値 |

## 3. テーブル定義（案）

### day

| 列 | 型 | 説明 |
| --- | --- | --- |
| date | TEXT PK | 論理日付 `YYYY-MM-DD` |
| tz | TEXT | 記録時のタイムゾーン |
| carryover_done_at | TEXT NULL | その日の持ち越し処理を終えた時刻 |
| created_at | TEXT | 作成時刻（ISO 8601） |

### item

| 列 | 型 | 説明 |
| --- | --- | --- |
| id | TEXT PK | ULID |
| date | TEXT FK→day | 所属する日 |
| kind | TEXT | `todo` / `blocker` |
| text | TEXT | 1行の本文 |
| status | TEXT | todo: `open` / `done` / `dropped`、blocker: `open` / `resolved` |
| close_reason | TEXT NULL | `status = dropped` の理由：`dropped`（やめる）/ `split`（分割した）/ `returned_external`（外部ツールへ戻した） |
| sort_key | TEXT | 並び順（分数インデックス） |
| carry_count | INTEGER | 持ち越された回数（0 から。持ち越すたびに元項目の値＋1） |
| carried_from_id | TEXT NULL | 持ち越し元（前回の日）の項目の id |
| split_from_id | TEXT NULL | 分割元の項目の id（分割で作った子項目のみ） |
| resolution_note | TEXT NULL | 障害の解消メモ |
| origin | TEXT | `human` / `agent` / `note`（ノートから送った） / `external` |
| agent_client_id | TEXT NULL | エージェントが作成した場合の識別 |
| external_ref_id | TEXT NULL | 外部課題との紐付け |
| completed_at | TEXT NULL | 完了・解消時刻 |
| created_at / updated_at / deleted_at | TEXT | 監査用時刻 |

持ち越しは「前回の日の項目を移動する」のではなく、「今日に新しい項目を作り、`carried_from_id` で元を指す」方式とする。前回の日とは、今日より前で項目またはノートが存在する直近の日であり、それより前の日は持ち越しの対象にしない。過去日のボードが当時の状態のまま残り、振り返りと検索の信頼性が保たれる。

- 前回の日の元項目の `status` は変更しない。持ち越し済みかどうかは、`carried_from_id` で元を指す項目があるかで判定する（`carried_to_id` は派生列またはビューで参照する）。
- 見直しで「やめる」を選んだ TODO は、元項目を `status = dropped`、`close_reason = dropped` にする。「外部ツールへ戻す」（外部連携が有効で、外部課題と紐付いた TODO のみ）は `close_reason = returned_external` とする。
- 「分割する」を選んだ場合は、元項目を `status = dropped`、`close_reason = split` にし、今日に作る子項目は `split_from_id` で元を指す（子項目の `carry_count` は 0）。

### note / reflection

| 列 | 型 | 説明 |
| --- | --- | --- |
| date | TEXT PK FK→day | 論理日付 |
| body | TEXT | Markdown 風のプレーンテキスト |
| updated_at | TEXT | 最終更新時刻 |

ノートの会議区切りは本文中の見出し行（`## 10:00 定例` 形式）で表し、別テーブルは設けない。見出しの一覧は本文から都度抽出する。

### note_send

| 列 | 型 | 説明 |
| --- | --- | --- |
| id | TEXT PK | ULID |
| date | TEXT | ノートの日付 |
| line_text | TEXT | 送った時点の行の本文 |
| line_fingerprint | TEXT | 行の位置変化に追随するための指紋（本文のハッシュと前後の行の情報） |
| item_id | TEXT FK→item | 送り先の項目 |
| created_at | TEXT | 送った時刻 |

送信済みの印は、表示時に本文の各行を `line_fingerprint` と照合して付ける。本文が一致すれば行の位置が変わっても印を保つ。本文を編集して一致しなくなった場合は印を外す（送り先の項目は残る）。この挙動は、人による編集とエージェントの `edit_note` で同じとする。複数行を選んで送った場合は、1行ごとに1件の項目と1件の note_send を作る。

### change_log

| 列 | 型 | 説明 |
| --- | --- | --- |
| id | TEXT PK | ULID |
| actor | TEXT | `agent` / `human`（取り消しを人が実行した場合などは `human`） |
| agent_client_id | TEXT FK NULL | 操作したエージェント（CLI は `cli`）。`actor = human` のときは NULL |
| operation | TEXT | `<名詞>.<動詞>` 形式。例：`todo.add`、`todo.complete`、`note.edit`（MCP・CLI 仕様書 3章の命名規則） |
| target_type / target_id | TEXT | 対象。target_type は `item` / `note` / `reflection` |
| before / after | TEXT（JSON） | 変更前後のスナップショット |
| created_at | TEXT | 操作時刻 |
| reverted_at | TEXT NULL | 取り消した時刻 |
| revert_of_id | TEXT NULL | この記録が取り消し操作である場合、取り消した元の記録の id |
| seen_at | TEXT NULL | 利用者が変更履歴画面で確認した時刻 |

取り消しは `before` を書き戻す新しい操作として実行し、取り消し自体も `revert_of_id` 付きで記録する。対象がその後に人によって変更されている場合は、取り消し前に差分を示して確認を求める。

画面の「エージェントによる未確認の変更」の印（上部のバーの印・項目の行頭の菱形）は、`seen_at` が NULL の記録がある項目に付ける。

### agent_client

| 列 | 型 | 説明 |
| --- | --- | --- |
| id | TEXT PK | ULID |
| name | TEXT | 例：`claude-code`、`codex`、`cli`、`unknown`（識別子のない MCP 接続） |
| scopes | TEXT（JSON） | `read` / `todo` / `blocker` / `append` / `note_edit` の許可。既定はどのクライアントも `read` のみ |
| created_at / last_seen_at | TEXT | 登録・最終接続時刻 |

### external_ref（MVP 後の有料機能）

| 列 | 型 | 説明 |
| --- | --- | --- |
| id | TEXT PK | ULID |
| provider | TEXT | `linear` / `jira` |
| external_id | TEXT | 課題キー（例：`ENG-123`） |
| url / title | TEXT | 表示用 |
| state / state_category | TEXT | 外部での状態と分類 |
| fetched_at | TEXT | 最終取得時刻 |

### setting

キーと値（JSON）の組で保持する。主なキー：`day_boundary`（既定 `04:00`）、`layout`、`theme`（`system` / `light` / `dark`。既定 `system`）、`font_size`、`vim_mode`、`shortcuts`、`locale`、`carryover_review_threshold`（既定 3）、`update_check`（更新の自動確認。既定 `true`）、`meter_hours`（凪メーターの時間帯。既定 `09:00-19:00`）、`sidebar`（サイドバーの開閉の状態）。

保存先（`data_dir`）は DB には持たず、設定ファイル `config.json` で管理する（アーキテクチャ設計書 6章）。エージェントの権限は agent_client テーブルで管理する。

## 4. 全文検索

FTS5 の仮想テーブル `search_index(kind, ref_id, date, body)` を用意し、item・note・reflection の更新時にトリガーで同期する（`deleted_at` が設定された項目は索引から外す）。日本語は単語区切りがないため、trigram トークナイザを採用する。trigram は3文字未満の検索語に索引を使えないため、2文字以下の検索語は元テーブル（item.text、note.body、reflection.body）に対する `LIKE '%語%'` の部分一致検索で扱う。5年分の規模で1秒以内に収まるかはテスト計画書の性能テストで確認する。

## 5. 容量の見積もり

1日あたり TODO・障害 20件、ノート 10KB、振り返り 2KB と仮定すると、5年（約1,800日）で本文は約 25MB、索引を含めて 100MB 未満に収まる見込み。要件の「5年分で検索1秒以内」はこの規模で検証する。

## 6. 未決事項

- バックアップ設定（世代数など）を setting に持つか（要件 F-12 の「バックアップ」の範囲）
- クラウド同期で採用されなかった競合内容の保存先（同期用の新しい表を設けるか、change_log を人・同期操作にも広げるか。クラウド同期仕様書と合わせて決める）
