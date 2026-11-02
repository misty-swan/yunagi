# MCP・CLI 仕様書

## 1. 概要

Yunagi は、AIコーディングエージェント（Claude Code、Codex など）とシェルから使える2つの接続口を提供する。

- CLI：コンソール用バイナリ `yunagi`（Windows は `yunagi.exe`）のサブコマンド群。
- MCP サーバー：`yunagi mcp` で起動する stdio 方式の MCP サーバー。

両者は同じ操作を同じ名前で持ち（3章の命名規則）、同じ権限モデルに従う。いずれもアプリ起動中はローカル API 経由でコアに依頼し、停止中は操作ごとにロックを取ってデータベースを直接開く（アーキテクチャ設計書 4章）。Yunagi の接続口自体は端末の外へデータを送らない。ただし、読み取りを許可されたエージェントは、読み取った内容をそのエージェントの仕組みに従って AI サービスへ送ることがある。AIの利用料は各AIサービスとの利用者の契約に含まれ、Yunagi の接続口は無料で提供する。

例外として、A-07（導入）と A-08（権限・履歴）は利用者が行う管理操作のため、CLI とアプリにだけ置き、MCP には出さない（エージェントが自分の権限を変えられないようにするため）。

## 2. 権限（スコープ）

| スコープ | 許可される操作 | 既定 |
| --- | --- | --- |
| read | ボード・項目・ノート・振り返りの取得、検索 | 有効 |
| todo | TODO の追加・編集・完了・取り消し | 無効 |
| blocker | 障害の追加・解消 | 無効 |
| append | 振り返りとノートの末尾への追記 | 無効 |
| note_edit | ノートの既存行の書き換え・削除・並べ替え | 無効 |

- 権限はエージェント（agent_client）ごとに設定する。既定はどのクライアントも `read` のみ。
- CLI からの操作は、人が実行したかエージェントが実行したかにかかわらず `cli` クライアントとして扱い、change_log に記録する。`cli` の既定スコープも `read` のみ。
- CLI は人とエージェントを確実には区別できず、防御の境界ではない（エージェントはシェル経由で CLI を呼べる）。そのため、スコープの変更と `yunagi agent revert` は、アプリの設定画面か、対話端末（TTY）上の確認プロンプトでしか確定できない。非対話の実行（エージェントのシェル、git フックなど）では確認を出さず、`permission_denied`（終了コード 3）を返す。
- エージェントの識別：`yunagi agent setup` は、登録するコマンドを `yunagi mcp --client claude-code` のように識別子付きで書き込む。識別子のない接続は `unknown` クライアントとして `read` のみを与える。
- 振り返りの既存の文章を書き換える操作は、どのスコープでも提供しない。
- 許可されていない操作は、エラーコード `permission_denied` と必要なスコープ名を返す。

## 3. 操作一覧（MVP：A-01〜A-08）

命名規則：MCP ツール名 `<動詞>_<名詞>` ＝ CLI `yunagi <名詞> <動詞>` ＝ change_log の operation `<名詞>.<動詞>`。例：`complete_todo` ＝ `yunagi todo complete` ＝ `todo.complete`。CLI には短い別名を用意してよい（別名の列）。検索の `search` は名詞・動詞に分けない例外とする。

| 要件 | MCP ツール名 | CLI | CLI の別名 | スコープ |
| --- | --- | --- | --- | --- |
| A-01 | `get_board` | `yunagi board get [--date D]` | `yunagi board` | read |
| A-01 | `open_board` | `yunagi board open [--date D]`（未完了 TODO と未解消の障害のみ） | — | read |
| A-01 | `get_note` | `yunagi note get [--date D] [--heading H]` | `yunagi note show` | read |
| A-02 | `search` | `yunagi search QUERY [--from D] [--to D] [--kind K]` | — | read |
| A-03 | `add_todo` | `yunagi todo add TEXT [--date D]` | — | todo |
| A-03 | `edit_todo` | `yunagi todo edit ID TEXT` | — | todo |
| A-03 | `complete_todo` / `reopen_todo` | `yunagi todo complete ID` / `yunagi todo reopen ID` | `yunagi todo done ID` | todo |
| A-04 | `add_blocker` | `yunagi blocker add TEXT [--date D]` | — | blocker |
| A-04 | `resolve_blocker` | `yunagi blocker resolve ID [--note TEXT]` | — | blocker |
| A-05 | `append_reflection` | `yunagi reflection append TEXT [--date D]` | `yunagi reflect append` | append |
| A-05 | `append_note` | `yunagi note append TEXT [--date D] [--heading [TITLE]]` | — | append |
| A-05 | `edit_note` | `yunagi note edit --line N --expect TEXT (--replace TEXT \| --delete \| --move-to M) [--date D]` | — | note_edit |
| A-06 | `export_handoff` | `yunagi handoff export ID` | `yunagi handoff ID` | read |
| A-07 | —（MCP には出さない） | `yunagi agent setup [--claude-code] [--codex]` | — | — |
| A-08 | —（MCP には出さない） | `yunagi agent history` / `yunagi agent revert CHANGE_ID` / `yunagi agent permissions` | — | 閲覧は不要。変更・取り消しは 2章の確認が必要 |

- `D` は `YYYY-MM-DD`、`today`、`yesterday` を受け付ける。省略時は論理日付の今日。`--date` を持たない操作（ID を指定する操作）は、ID の項目が属する日を対象にする。
- 要件 A-03 の「取り消し」は、完了の取り消し（未完了に戻す＝`reopen_todo`）を指す。エージェントの変更の取り消し（A-08 の `agent revert`）とは別の操作である。
- `--kind K` は `todo` / `blocker` / `note` / `reflection` を受け付ける（複数指定可）。
- `append_note` の `--heading [TITLE]` は、現在時刻の見出し（`## HH:MM TITLE`）を挿入してから本文を追記する。
- `edit_note` の行番号 `N`・`M` は1始まり。`--expect`（MCP では `expected_text`）に書き換え前の行の本文を必ず渡し、一致しなければ `conflict` を返す（人の編集で行がずれた場合に別の行を書き換えないため）。
- A-01 のノート取得は、`get_board`（見出しと長さ）と `get_note`（本文）の組で満たす（4.2 参照）。

### 3.1 その他の CLI コマンド

| CLI | 内容 |
| --- | --- |
| `yunagi show [--note]` | アプリを前面に表示する（Wayland 環境でのショートカット割当にも使う） |
| `yunagi doctor` | 診断情報（版、OS、保存先、ローカル API の状態）を出力する。業務データの内容は含めない |
| `yunagi uninstall [--purge]` | アプリと CLI を削除する。`--purge` でデータも削除 |
| `yunagi --version` | 版を表示する |

## 4. 入出力

### 4.1 CLI

- 既定は人が読みやすいテキスト出力。`--json` で機械可読な JSON を出力する。
- 終了コード（4.3 のエラーコードとの対応）：

| 終了コード | 意味 | エラーコード |
| --- | --- | --- |
| 0 | 成功 | — |
| 1 | 一般エラー | 下記以外 |
| 2 | 引数エラー | `invalid_argument` |
| 3 | 権限エラー | `permission_denied` |
| 4 | 対象なし | `not_found` |
| 5 | 競合 | `conflict` |
| 6 | 保存先を利用できない | `storage_unavailable` |

- `TEXT` に `-` を指定すると標準入力から読む（例：`git log -1 --format=%s | yunagi reflection append -`）。

### 4.2 MCP ツールの返り値（例）

`get_board`：

```json
{
  "date": "2026-11-02",
  "todos": [
    {"id": "01J...", "text": "API のレート制限を調査", "status": "open", "carry_count": 1, "origin": "human", "external_ref": null}
  ],
  "blockers": [
    {"id": "01J...", "text": "ステージング環境の権限待ち", "status": "open", "carry_count": 0, "origin": "note", "external_ref": null, "resolution_note": null}
  ],
  "reflection": "…",
  "note": {"headings": ["10:00 定例", "14:00 設計レビュー"], "length": 2380}
}
```

ノート本文はサイズが大きくなりうるため、`get_board` では見出しと長さのみを返し、本文は `get_note`（read）で見出し単位または全文を取得する。

### 4.3 エラー

```json
{"error": {"code": "permission_denied", "required_scope": "todo", "message": "This agent is not allowed to modify TODOs. Enable it in Yunagi > Settings > Agents."}}
```

エラーコード：`permission_denied`、`not_found`、`invalid_argument`、`conflict`（対象が直前に変更された）、`storage_unavailable`。`message` の言語は未決（9章）。

## 5. 依頼文の書き出し（A-06）

選んだ TODO（1件）について、次の形式の Markdown を生成する。クリップボード（アプリ）または標準出力（CLI）へ出力する。

```markdown
## Task
API のレート制限を調査

## Context (from today's notes)
- 10:00 定例: 外部APIの429が増えている。上限は1分あたり600件らしい
- 10:00 定例: リトライは指数バックオフで

## Related blockers
- ステージング環境の権限待ち

(Exported from Yunagi on 2026-11-02)
```

関連するノートの行は、TODO がノートから送られた場合（note_send で対応する行がある場合）、送り元の行が属する時刻付き見出しの区間（見出しの次の行から、次の見出しの前まで）から抽出する。見出しがなければ、ノートの先頭から最初の見出しの前までとする。ノートから送られていない TODO では「Context」の節を省略する（操作仕様書 2章）。

## 6. 導入（A-07）

`yunagi agent setup` は、検出したエージェントの設定に MCP サーバーを登録する。

| 対象 | 登録方法（案） |
| --- | --- |
| Claude Code | `claude mcp add` 相当の登録（コマンド：`yunagi mcp --client claude-code`） |
| Codex | Codex の設定ファイルに MCP サーバー定義を追記（コマンド：`yunagi mcp --client codex`） |

各エージェントの設定形式は変わりうるため、実装時に最新の公式ドキュメントで確認する。登録前に変更内容を表示し、利用者の確認を得てから書き込む。既存の設定はバックアップする。

## 7. 変更履歴（A-08）

- todo・blocker・append・note_edit に該当する操作は、MCP・CLI のどちらからでも、すべて change_log に記録する（operation は 3章の命名規則に従う）。
- MCP・CLI から作成された項目には `origin = agent` と agent_client_id（CLI は `cli`）を記録し、画面で印を表示する。
- `yunagi agent revert` は取り消しを実行し、対象がその後に人によって変更されている場合は `conflict`（終了コード 5）を返す（`--force` で上書き）。取り消しの確定には 2章の確認が必要。

## 8. 第2フェーズで追加する操作（案。段階はロードマップ参照）

- エージェントの直接起動：`yunagi run ID --agent claude-code`
- フック連携：エージェントの作業終了時に要約を振り返りの下書きへ追記
- 変更の購読：`subscribe_changes`（MCP の通知機能を利用）

## 9. 未決事項

- エラーの `message` を利用者の言語設定（`locale`）に合わせて日英で出すか、英語固定にするか
- `cli` クライアントで作成した項目を、人が CLI で直接作ったものとして `origin = human` にする手段（対話端末での実行時など）を設けるか
