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
- `edit_note` の行番号 `N`・`M` は1始まり。`--expect`（MCP では `expected_text`）に書き換え前の行の本文を必ず渡し、一致しなければ `conflict`（`reason: changed`。4.3）を返す（人の編集で行がずれた場合に別の行を書き換えないため）。利用者が画面で入力中の行（アーキテクチャ設計書 4.1 の守る行）を書き換え・削除・移動しようとした場合、または画面から知らされた守る行が古い版のものである場合は `conflict`（`reason: user_editing`）を返す。両方に当たるときは `changed` を返す（`--expect` を先に確かめる）。`user_editing` のまま同じ操作をやり直しても、行が一致しない以上は必ず失敗するため、先に読み直させる。
- `resolve_blocker` は、同じ操作を何度送っても結果が同じになり、利用者の解消メモを黙って失わないように、次のとおり扱う（2026-11-02 決定）。エージェントは応答が届かなかったときなどに、同じ操作をそのままやり直してよい。
  - 状態：未解消の障害は解消にする。解消済みの障害は、エラーにせず、状態と解消時刻を変えない。
  - 解消メモ（`--note`。MCP では `note`）：状態とは別に、次のとおり扱う。障害の解消メモが空（NULL、または空白だけ）なら、`--note` の本文で置き換える。`--note` と同じ本文がすでにあれば書かない（未解消の障害なら、解消にはする）。別の本文があれば、上書きも追記もせず、`conflict`（`reason: note_exists`。4.3）を返す。上書きは利用者のメモを黙って消し、追記はやり直すたびにメモが重なるため。`note_exists` のときは操作を何も反映しない（未解消の障害でも解消にしない）。
  - 解消メモの比較：解消メモは1行（画面設計書 3章「→ メモ」、操作仕様書 2章）なので、コアは `--note` の前後の空白（改行・全角空白を含む Unicode の空白文字）を除いてから保存し、比較は除いた後の値と保存済みの値の完全一致とする。画面からの保存も同じく前後の空白を除く。除き方が一つに決まっていれば、同じ操作のやり直しも、画面で打った同じ本文も「同じ本文」と判定されるため。前後の空白を除いて空になる `--note` と、途中に改行を含む `--note` は `invalid_argument` を返す。上の「空白だけ」も同じ空白文字で判定する。
  - 利用者の入力中：`--note` が付いていて、利用者がその障害の解消メモを入力中（アーキテクチャ設計書 4.1 の定義。フォーカスがあり、最後の入力から 10 秒以内か IME の変換中）なら、`conflict`（`reason: user_editing`）を返す。入力が終われば、やり直した操作は上の判定に従う。障害を解消した操作（Mod+Enter・x・チェック欄のクリック）は、解消メモの欄への入力に数えない。入力中でなければ、解消メモの欄にフォーカスがあっても書き込み、その場で画面に反映する（`edit_todo` と同じ。解消した直後でまだ何も打っていない欄も含む）。カーソルは解消メモの末尾に置く。利用者はその内容を見たうえで書き換えられ、書き換えた場合は 4.1「項目の保存」のとおり人の変更として扱う。
  - 判定の順：`permission_denied`・`not_found`・`invalid_argument`・`storage_unavailable` を先に確かめ、続けて `user_editing`、`note_exists` の順に確かめる。どれにも当たらなければ成功とする。そのため、何も変えない操作でも、保存の再試行中（アーキテクチャ設計書 4.1）は `storage_unavailable` になる。
  - 応答と記録：成功の応答には、実際に何かを変えたかを示す `updated`（`true` / `false`）を含める。エージェントが、自分が解消したのか、すでに解消されていたのかを区別して利用者に伝えられるようにするため。CLI では、`--json` のときに同じ `updated` を含む JSON を出し、テキスト出力では `Resolved.`（解消した）、`Note added.`（解消メモだけを書いた）、`Already resolved. Nothing changed.`（何も変えていない）を表示する。終了コードはどれも 0。何も変えなかった場合（`updated: false`）は、change_log に記録しない（7章）。記録するときは、`blocker.resolve` として実際に変えた欄だけを `before` / `after` に入れる（解消にした場合は状態と解消時刻、解消メモを書いた場合は解消メモ。同じ本文がすでにあって書かなかった解消メモは入れない）。
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
{"error": {"code": "conflict", "reason": "user_editing", "message": "The user is editing this item. Try again later."}}
```

エラーコード：`permission_denied`、`not_found`、`invalid_argument`、`conflict`、`storage_unavailable`。`storage_unavailable` は、画面での保存の書き込みに失敗して再試行している間、対象に関係なくすべての書き込みに返す（読み取りは受け付ける。アーキテクチャ設計書 4.1）。

`conflict` には、エージェントが次の動きを決められるよう、理由を `reason` 欄で付ける。CLI の終了コードはどの理由でも 5 で、`--json` のときは同じ JSON を出し、テキスト出力では `message` を表示する。

| `reason` | 意味 | エージェントの次の動き |
| --- | --- | --- |
| `changed` | 対象が直前に変更された（`edit_note` の `--expect` の不一致、`agent revert` の対象がその後に人に変更されている場合） | 最新の内容を読み直してから、操作を組み立て直す。`agent revert` は人が実行する操作で、差分を確かめたうえで `--force` で上書きもできる（7章） |
| `note_exists` | `resolve_blocker` の `--note` と異なる解消メモが、障害にすでにある（3章）。操作は何も反映していない | 最新の内容を読み直す。エージェントは既存の解消メモを書き換えられないため、足したい内容があれば利用者に伝える。解消だけが目的なら `--note` を付けずに送る（同じ `--note` のままやり直しても必ず失敗する） |
| `user_editing` | 利用者が対象を入力中（画面から知らされた守る行が古い版のもので、知らせ直しを待っている場合を含む。アーキテクチャ設計書 4.1）。操作は何も反映していない | 同じ操作を、しばらくしてからそのままやり直す |

`user_editing` の場合は、`message` でもその旨と、しばらくしてからやり直すよう伝える。文言は対象に合わせる（例：項目なら `The user is editing this item. Try again later.`、ノート・振り返りの行なら `The user is editing this line. Try again later.`）。`note_exists` の場合は、やり直しても成功しないことと次の動きを伝える（例：`This blocker already has a different resolution note. It was not changed. To resolve it, send the request without a note.`）。`message` の言語は未決（9章）。

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

関連するノートの行は、TODO がノートから送られた場合（note_send で対応する行がある場合）、送り元の行が属する時刻付き見出しの区間（見出しの次の行から、次の見出しの前まで）から抽出する。見出しがなければ、ノートの先頭から最初の見出しの前までとする。ノートから送られていない TODO では「Context」の節を省略する（操作仕様書 2章）。持ち越しや分割で作られた TODO は、元の項目へさかのぼって送り元の行を探す（データモデル設計書 3章 note_send）。

「Related blockers」には、同じ見出しの区間の行からノートで送った障害のうち、未解消のものを載せる。該当する障害がない場合と、ノートから送られていない TODO では、この節を省略する。

## 6. 導入（A-07）

`yunagi agent setup` は、検出したエージェントの設定に MCP サーバーを登録する。

| 対象 | 登録方法（案） |
| --- | --- |
| Claude Code | `claude mcp add` 相当の登録（コマンド：`yunagi mcp --client claude-code`） |
| Codex | Codex の設定ファイルに MCP サーバー定義を追記（コマンド：`yunagi mcp --client codex`） |

各エージェントの設定形式は変わりうるため、実装時に最新の公式ドキュメントで確認する。登録前に変更内容を表示し、利用者の確認を得てから書き込む。既存の設定はバックアップする。

## 7. 変更履歴（A-08）

- todo・blocker・append・note_edit に該当する操作は、MCP・CLI のどちらからでも、すべて change_log に記録する（operation は 3章の命名規則に従う）。ただし、何も変えずに成功を返した操作（解消済みの障害への `resolve_blocker` など。3章）は記録しない。
- 利用者が画面で項目の状態を変えてから保存されるまでの間に届いた、同じ状態にする `complete_todo`・`reopen_todo`・`resolve_blocker` は、受け付けて成功を返すが、利用者の保存が書き込まれた時点で利用者の変更として扱い、その操作の記録から状態と完了時刻の欄を外す（何も残らなければ記録を消す。アーキテクチャ設計書 4.1）。利用者の保存が先に届いた場合に記録されないのと同じ結果にするため。
- MCP・CLI から作成された項目には `origin = agent` と agent_client_id（CLI は `cli`）を記録し、画面で印を表示する。
- `yunagi agent revert` は取り消しを実行し、対象がその後に人によって変更されている場合は `conflict`（`reason: changed`。終了コード 5）を返す（`--force` で上書き）。項目では、その記録の操作が変えた欄だけで判定し、取り消しもその欄だけを戻す（ほかの欄に人が加えた変更は残る。データモデル設計書 change_log）。取り消しの確定には 2章の確認が必要。

## 8. 第2フェーズで追加する操作（案。段階はロードマップ参照）

- エージェントの直接起動：`yunagi run ID --agent claude-code`
- フック連携：エージェントの作業終了時に要約を振り返りの下書きへ追記
- 変更の購読：`subscribe_changes`（MCP の通知機能を利用）

## 9. 未決事項

- エラーの `message` を利用者の言語設定（`locale`）に合わせて日英で出すか、英語固定にするか
- `cli` クライアントで作成した項目を、人が CLI で直接作ったものとして `origin = human` にする手段（対話端末での実行時など）を設けるか
