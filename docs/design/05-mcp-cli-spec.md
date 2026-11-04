# MCP・CLI 仕様書

## 1. 概要

Yunagi は、AIコーディングエージェント（Claude Code、Codex など）とシェルから使える2つの接続口を提供する。

- CLI：コンソール用バイナリ `yunagi`（Windows は `yunagi.exe`）のサブコマンド群。
- MCP サーバー：`yunagi mcp` で起動する stdio 方式の MCP サーバー。

両者は同じ操作を同じ名前で持ち（3章の命名規則）、同じ権限モデルに従う。いずれもアプリ起動中はローカル API 経由でコアに依頼し、停止中は操作ごとにロックを取ってデータベースを直接開く（アーキテクチャ設計書 4章）。どちらで処理するかは、操作ごとに、まずローカル API に接続を試み、無いときだけ直接モードにして決める。ロックを取れなかったときは `busy` を返す（アーキテクチャ設計書 4.4、本書 4.3）。Yunagi の接続口自体は端末の外へデータを送らない。ただし、読み取りを許可されたエージェントは、読み取った内容をそのエージェントの仕組みに従って AI サービスへ送ることがある。AIの利用料は各AIサービスとの利用者の契約に含まれ、Yunagi の接続口は無料で提供する。

例外として、A-07（導入）と A-08（権限・履歴）は利用者が行う管理操作のため、CLI とアプリにだけ置き、MCP には出さない（エージェントが自分の権限を変えられないようにするため）。

## 2. 権限（スコープ）

| スコープ | 許可される操作 | 既定 |
| --- | --- | --- |
| read | ボード・項目・ノート・振り返りの取得、検索 | 有効 |
| todo | TODO の追加・編集・完了・取り消し | 無効 |
| blocker | 障害の追加・編集・解消 | 無効 |
| append | 振り返りとノートの末尾への追記 | 無効 |
| note_edit | ノートの既存行の書き換え・削除・並べ替え | 無効 |

- 権限はエージェント（agent_client）ごとに設定する。既定はどのクライアントも `read` のみ。
- CLI からの操作は、人が実行したかエージェントが実行したかにかかわらず `cli` クライアントとして扱い、change_log に記録する。`cli` の既定スコープも `read` のみ。
- CLI は人とエージェントを確実には区別できず、防御の境界ではない（エージェントはシェル経由で CLI を呼べる）。そのため、スコープの変更と `yunagi agent revert` は、アプリの設定画面か、対話端末（TTY）上の確認プロンプトでしか確定できない。非対話の実行（エージェントのシェル、git フックなど）では確認を出さず、`permission_denied`（終了コード 3）を返す。
- エージェントの識別：`yunagi agent setup` は、登録するコマンドを `yunagi mcp --client claude-code` のように識別子付きで書き込む。識別子のない接続は `unknown` クライアントとして `read` のみを与える。
- 振り返りの既存の文章を書き換える操作は、どのスコープでも提供しない。
- 許可されていない操作は、エラーコード `permission_denied` と必要なスコープ名を返す。
- 権限はコアが確かめる（アーキテクチャ設計書 4章 5）ので、アプリ経由（ローカル API）でも直接モードでも同じになる。書き込みだけでなく読み取りも同じで、いま作ってある `get_board`・`open_board`・`get_note` は、コアの関数が呼び出し元のクライアントを受け取り、はじめに `read` を確かめる（2026-11-04 決定。コアの `domain::board`・`domain::text_doc`）。画面が使う読み取り（`get_screen_board`、`text_doc::get`）はエージェントの操作ではないので確かめない。登録されていないクライアントには何も許可しない（`read` も含む）。`search`・`export_handoff` の入り口は、作るときに同じ形（呼び出し元を受け取り、はじめに `read` を確かめる）にする（YUN-90 の完了の条件に書き足した）。

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
| A-04 | `edit_blocker` | `yunagi blocker edit ID TEXT` | — | blocker |
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
- `edit_note` の行番号 `N`・`M` は1始まり。`--expect`（MCP では `expected_text`）に書き換え前の行の本文を必ず渡し、一致しなければ `conflict`（`reason: changed`。4.3）を返す（人の編集で行がずれた場合に別の行を書き換えないため）。利用者が画面で入力中の行（アーキテクチャ設計書 4.1 の守る行）を書き換え・削除・移動しようとした場合、または画面から知らされた守る行が古い版のものである場合は `conflict`（`reason: user_editing`）を返す。両方に当たるときは `changed` を返す（`--expect` を先に確かめる）。守る行を本文を変えずに書き換える場合も `user_editing` を返す（入力中の行はまだ保存されておらず、利用者の見ている行と違いうるため。項目の本文の `edit_todo` と同じ）。守る行ではない行の移動で、守る行の番号がずれるだけの場合は受け付ける（画面は差分として取り込む。2026-11-03 決定）。`user_editing` のまま同じ操作をやり直しても、行が一致しない以上は必ず失敗するため、先に読み直させる。
- `resolve_blocker` は、同じ操作を何度送っても結果が同じになり、利用者の解消メモを黙って失わないように、次のとおり扱う（2026-11-02 決定）。エージェントは応答が届かなかったときなどに、同じ操作をそのままやり直してよい。
  - 状態：未解消の障害は解消にする。解消済みの障害は、エラーにせず、状態と解消時刻を変えない。
  - 解消メモ（`--note`。MCP では `note`）：状態とは別に、次のとおり扱う。障害の解消メモが空（NULL、または空白だけ）なら、`--note` の本文で置き換える。`--note` と同じ本文がすでにあれば書かない（未解消の障害なら、解消にはする）。別の本文があれば、上書きも追記もせず、`conflict`（`reason: note_exists`。4.3）を返す。上書きは利用者のメモを黙って消し、追記はやり直すたびにメモが重なるため。`note_exists` のときは操作を何も反映しない（未解消の障害でも解消にしない）。
  - 解消メモの比較：解消メモは1行（画面設計書 3章「→ メモ」、操作仕様書 2章）なので、コアは `--note` の前後の空白（改行・全角空白を含む Unicode の空白文字）を除いてから保存し、比較は、除いた後の値と、保存済みの値から同じく除いた後の値の完全一致とする。画面からの保存も同じく前後の空白を除く。除き方が一つに決まっていれば、同じ操作のやり直しも、画面で打った同じ本文も「同じ本文」と判定されるため。保存済みの値はいつも除いた後の形だが、保存済みの側も除くのは、外から入った値でもやり直しが `note_exists` にならないようにするため。`note_exists` で返す解消メモも除いた形にする（2026-11-03 決定。データモデル設計書 3章 item）。前後の空白を除いて空になる `--note` と、途中に改行を含む `--note` は `invalid_argument` を返す。上の「空白だけ」も同じ空白文字で判定する。
  - 利用者の入力中：`--note` が付いていて、利用者がその障害の解消メモを入力中（アーキテクチャ設計書 4.1 の定義。フォーカスがあり、最後の入力から 10 秒以内か IME の変換中）なら、`conflict`（`reason: user_editing`）を返す。入力が終われば、やり直した操作は上の判定に従う。障害を解消した操作（Mod+Enter・x・チェック欄のクリック）は、解消メモの欄への入力に数えない。入力中でなければ、解消メモの欄にフォーカスがあっても書き込み、その場で画面に反映する（`edit_todo` と同じ。解消した直後でまだ何も打っていない欄も含む）。カーソルは解消メモの末尾に置く。利用者はその内容を見たうえで書き換えられ、書き換えた場合は 4.1「項目の保存」のとおり人の変更として扱う。
  - 判定の順：3.2 のとおり。`user_editing` は `--note` 付きのときだけ確かめ、最後に `note_exists` を確かめる。
  - 応答と記録：成功の応答と記録は 3.2 のとおり。`updated` で、エージェントが、自分が解消したのか、すでに解消されていたのかを区別して利用者に伝えられる。CLI のテキスト出力では `Resolved.`（解消した）、`Note added.`（解消メモだけを書いた）、`Already resolved. Nothing changed.`（何も変えていない）を表示する。記録の operation は `blocker.resolve` で、実際に変えた欄（解消にした場合は状態と解消時刻、解消メモを書いた場合は解消メモ。同じ本文がすでにあって書かなかった解消メモは入れない）だけを入れる。
- 解消済みの障害を画面で未解消に戻しても、解消メモは消さずに残す（操作仕様書 2章）。そのため、未解消の障害にも解消メモがあることがあり、その障害への `--note` 付きの `resolve_blocker` は上の解消メモの判定に従う（別の本文があれば `note_exists`）。エージェントは `get_board` などの `resolution_note` で残っている解消メモを確かめられる。未解消の間は画面に解消メモが出ないため、未解消の障害で `note_exists` になったときは、前に解消したときの解消メモが残っていることを、その本文とともに利用者に伝える。`--note` を付けずに解消すると、残っていた解消メモがそのまま解消済みの障害の下に表示される。そのため `resolve_blocker` の成功の応答には、操作の後の `resolution_note` を含める。エージェントは、`--note` を付けずに解消した障害に解消メモが残っていたら、その本文とともに利用者に伝える。
- `complete_todo`・`reopen_todo` も、`resolve_blocker` と同じく、同じ操作を何度送っても結果が同じになるように扱う（2026-11-03 決定。3.2）。
  - 状態：`complete_todo` は未完了（`open`）の TODO を完了（`done`）にし、完了時刻を入れる。`reopen_todo` は完了の TODO を未完了に戻し、完了時刻を空（NULL）にする。すでに目的の状態の TODO（`complete_todo` なら完了、`reopen_todo` なら未完了）には、3.2 の何も変えない成功を返す（状態と完了時刻を変えない。完了時刻を変えないのは、最初に完了した時刻を残すため）。
  - 対象：ID が障害を指す場合と、取りやめた TODO（`dropped`。持ち越しの見直しで「やめる」「分割する」「外部ツールへ戻す」を選んだもの。データモデル設計書 3章）を指す場合は、どちらの操作も何も変えずに `invalid_argument` を返す。取りやめた TODO を完了や未完了に変えると、利用者が閉じた項目を黙って戻すことになるため。同じく、`resolve_blocker` に TODO の ID を渡した場合も `invalid_argument` を返す（取りやめた TODO の ID なら、`complete_todo` で送り直しても成功しないため、種類の違いではなく取りやめた TODO のエラーを返す。下記の `edit_blocker` と同じ。2026-11-03 決定）。`message` では、やり直しても成功しないことと理由を伝える（例：`This TODO was dropped by the user. It was not changed. Do not retry.`、`complete_todo` に障害の ID なら `This ID is a blocker, not a TODO. It was not changed. Use resolve_blocker.`）。種類の違いのエラーの中身（`item_kind`・`use_operation`）は、下記の `edit_todo` に障害の ID が届いた場合と同じ。エージェントは同じ操作をやり直さず、取りやめた TODO なら利用者に伝える。
  - 判定の順：3.2 のとおり。状態の欄は入力中でも受け付けるので `user_editing` は返さない（アーキテクチャ設計書 4.1）。
  - 応答と記録：成功の応答と記録は 3.2 のとおり。CLI のテキスト出力では `Completed.`（完了にした）・`Already done. Nothing changed.`（`complete_todo` で何も変えていない）、`Reopened.`（未完了に戻した）・`Already open. Nothing changed.`（`reopen_todo` で何も変えていない）を表示する。記録の operation は `todo.complete` / `todo.reopen` で、状態と完了時刻を入れる。
- 取りやめた TODO（`dropped`）への `edit_todo` も、`complete_todo`・`reopen_todo` と同じく、本文を何も変えずに `invalid_argument` を返す（2026-11-03 決定）。取りやめた TODO は、利用者が持ち越しの見直しで閉じた項目で、その日の終わりの記録として本文も残す（データモデル設計書 3章。持ち越し済みの項目の本文を変えないのと同じ）。分割した TODO（`close_reason=split`）なら、子の TODO がその本文をもとに作られており、元の本文を変えると子との対応が分からなくなる。また、取りやめた TODO は今日のボードに無いため、そこへ届いた `edit_todo` は、エージェントが古いボードを読んで対象を取り違えたものである見込みが高い。
  - 送られた本文が今の本文と同じでも、`updated: false` の成功にはせず `invalid_argument` を返す（送った本文に関係なく、取りやめた TODO の本文は変えられないことを、どの送り方でも同じ結果で伝えるため）。`message` は `complete_todo`・`reopen_todo` と同じく、やり直しても成功しないことと理由を伝える（例：`This TODO was dropped by the user. It was not changed. Do not retry.`）。エージェントは同じ操作をやり直さず、本文を変えたかった理由とともに利用者に伝える。
  - 取りやめた TODO への `edit_todo`・`complete_todo`・`reopen_todo` のエラーには、取りやめの理由 `close_reason`（`dropped` / `split` / `returned_external`。データモデル設計書 3章）を含め、`message` もそれに合わせる（例：`split` なら `This TODO was split into other TODOs by the user. It was not changed. Do not retry.`）。分割した・外部へ戻した TODO を、エージェントが「やめた」と利用者に伝えないようにするため。分割した TODO でも、子の TODO の id はエラーに含めない（子は複数ありうるため、どの子を直すべきかはエージェントには決められない。エージェントは利用者に伝える）。
  - 判定の順：3.2 の `invalid_argument` の最初に確かめる（本文を直してやり直しても成功しないことを、最初の応答で伝えるため）。取りやめた TODO は画面でも本文を入力できないので、`user_editing` には当たらない（操作仕様書 2章）。
  - 画面でも、取りやめた TODO の状態と本文は変えられない（操作仕様書 2章）。削除と並べ替えは通常どおりできる。
  - 障害には取りやめの状態が無い（データモデル設計書 3章）ので、この決まりは TODO だけに当てはまる。
- `edit_todo` に今と同じ本文が届いた場合も、`complete_todo`・`reopen_todo`・`resolve_blocker` と同じく、3.2 の何も変えない成功を返す（2026-11-03 決定）。
  - 比べ方：項目の本文も1行（データモデル設計書 item の `text`）なので、上の解消メモと同じ決まりで扱う。コアは、`edit_todo`・`edit_blocker`・`add_todo`・`add_blocker` の本文から、解消メモと同じ空白文字を前後から除いてから保存し、画面からの本文の保存も同じく除く（除いて空になる本文と、途中に改行を含む本文は `invalid_argument`）。改行は LF と CR のどちらも途中の改行とみなす（ノートの行と違い、項目の本文は画面で1行に表示するため。2026-11-03 決定）。比較は、送られた本文から除いた後の値と、保存済みの本文から除いた後の値の完全一致とする（保存済みの本文はいつも除いた後の形だが、解消メモと同じく保存済みの側も除く。2026-11-03 決定）。一致すれば本文を書かない。CLI の `TEXT` に `-` を指定して標準入力から読んだ本文（4.1）の末尾の改行も除かれるので、同じコマンドのやり直しは同じ本文と判定される。
  - 欄ごとの扱い：`edit_todo` が変える欄は、いまは本文だけである。後で欄を足した場合（MVP 後）も、change_log に変えた欄だけを入れる決まり（データモデル設計書 change_log）に合わせて、送られた欄ごとに今の値と比べ、違う欄だけを書く。一部の欄だけが同じ（本文は同じで、別の欄は違う）なら、違う欄だけを書いて `updated: true` で成功し、記録にもその欄だけを入れる。すべての欄が同じときだけ `updated: false` にする（`resolve_blocker` で、同じ本文がすでにあって書かなかった解消メモを記録に入れないのと同じ）。
  - 判定の順：3.2 のとおり（`user_editing` は、その項目の本文を入力中のとき）。入力中を同じ本文の判定より先にするのは、入力中は画面の本文がまだ保存されておらず、保存済みの本文と同じでも、利用者の見ている本文とは違うことがあるため。
  - 応答と記録：成功の応答と記録は 3.2 のとおり。応答には、操作の後の本文 `text` も含める（前後の空白を除いて保存した本文を、エージェントが読み直さずに確かめられるようにするため。CLI の `--json` も同じ）。CLI のテキスト出力では `Edited.`（本文を変えた）・`Same text. Nothing changed.`（何も変えていない）を表示する。記録の operation は `todo.edit` で、実際に変えた欄だけ（いまは本文だけ）を入れる。
- `edit_todo` に障害の ID が届いた場合は、本文を何も変えずに `invalid_argument` を返す。障害の本文は、新しく設ける `edit_blocker`（`blocker` スコープ）で変える（2026-11-03 決定）。`complete_todo`・`reopen_todo` に障害の ID を、`resolve_blocker` に TODO の ID を渡した場合と同じく、操作の名詞と項目の種類をそろえるため（命名規則）。`edit_todo` で障害を受け付けると、`todo` スコープだけのエージェントが障害の本文を変えられ、`blocker` スコープだけのエージェントは自分で足した障害の本文も直せなくなる（2章）。change_log にも、障害の変更が `todo.edit` として残る。
  - `edit_blocker` の扱い：障害の本文を変える。今と同じ本文の扱い（上の `edit_todo` の「比べ方」「欄ごとの扱い」「応答と記録」）は `edit_todo` と同じで、記録の operation は `blocker.edit` とする。解消済みの障害も、未解消の障害と同じく受け付ける（完了済みの TODO への `edit_todo` と同じ）。障害の解消メモの入力中は、本文とは別の欄なので受け付ける（アーキテクチャ設計書 4.1）。持ち越し済みの障害の扱いは下記。
  - 判定の順：3.2 のとおり（`edit_todo` と同じ並びで、スコープは `blocker`）。種類の違う ID（`edit_todo` に障害、`edit_blocker` に TODO）を、本文が空などのほかの `invalid_argument` より先に確かめるのは、本文を直しても、時間をおいても成功しないことを、最初の応答で伝えるため。そのため、`carried_over` が示す持ち越し先は、いつも送った操作と同じ種類の項目になる。
  - スコープは、種類の違いより先に、送られた操作のスコープで確かめる（3.2）。そのため、`todo` スコープを持たないエージェントの `edit_todo` は、ID が障害でも `permission_denied` になる。エージェントは、`get_board` などの `todos` / `blockers` のどちらに載っている ID かで操作を選ぶ。`permission_denied` を受けて利用者にスコープの許可を頼む前にも、そうして操作が合っているかを確かめる（使わない操作のスコープを利用者に許可させないため）。
  - エラーの中身：種類の違いの `invalid_argument` には、項目の種類 `item_kind`（`todo` / `blocker`）と、その項目に使う操作 `use_operation`（対応する操作が無ければ `null`）を含める。`message` の文言（言語は未決。9章）に頼らずに、エージェントが送り直す先を決められるようにするため。送られた本文が今の本文と同じでも、`updated: false` の成功にはせず同じエラーを返す（取りやめた TODO への `edit_todo` と同じく、送った本文に関係なく同じ結果で伝えるため）。`message` は、何も変えていないことと、使う操作を伝える（例：`This ID is a blocker, not a TODO. It was not changed. Use edit_blocker.`、`This ID is a TODO, not a blocker. It was not changed. Use edit_todo.`）。エージェントは同じ操作をやり直さず、`use_operation` のスコープを持っていれば同じ ID にその操作で送り直し、持っていなければ利用者に伝える。
  - `edit_blocker` に取りやめた TODO の ID を渡した場合は、`edit_todo` で送り直しても成功しないため、種類の違いのエラーではなく、取りやめた TODO への `edit_todo` と同じエラー（`close_reason` を含み、`message` もそれに合わせる。上記）を返す。
  - `complete_todo`・`reopen_todo`・`resolve_blocker` の種類の違い（上記）のエラーにも、同じく `item_kind`・`use_operation` を含める。`use_operation` は、`complete_todo` に障害なら `resolve_blocker`、`resolve_blocker` に TODO なら `complete_todo`、`reopen_todo` に障害なら `null`（解消済みの障害を未解消に戻す操作はエージェントに無く、画面でだけ行える。操作仕様書 2章）。`null` のときの `message` は、使う操作を示さずに利用者に伝えるよう促す（例：`This ID is a blocker, not a TODO. It was not changed. Agents cannot reopen blockers; ask the user.`）。
- 持ち越し済みの項目（持ち越し元。データモデル設計書 3章「持ち越し」）への書き込みは、持ち越し先へ向け直さず、何も変えずに拒む（2026-11-03 決定）。持ち越すと前の日の項目は `open` のまま残るため、そのまま受け付けると、前の日の項目だけが完了などになり、今日の持ち越しの項目は元のまま残って、利用者とエージェントの見ている状態が食い違うため。黙って向け直すと、エージェントが指定した項目とは別の項目が変わり、`edit_todo` の本文や解消メモの判定も別の項目で行われるため、向け直しはしない。
  - 対象の操作：`edit_todo`・`edit_blocker`・`complete_todo`・`reopen_todo`・`resolve_blocker`。ID が持ち越し済みの項目を指す場合は、その項目の状態（完了・未完了など）にかかわらず、`conflict`（`reason: carried_over`。4.3）を返す。同じ操作を何度送っても同じ結果になり、前の日の項目は変わらない。
  - 持ち越し先の知らせ方：エラーに、持ち越し先の項目の id `carried_to_id`（データモデル設計書 3章の定義）と、その項目の日付 `carried_to_date` を含める。持ち越しが続いている場合（前の日から次の日へ、さらに今日へ）は、送り直した先でも `carried_over` が返るので、同じようにたどる。持ち越し先が取りやめた TODO（`dropped`。持ち越した次の日の見直しで「やめる」などを選んだもの）であることもあり、その ID に送った `edit_todo`・`complete_todo`・`reopen_todo` は上の `invalid_argument` の決まりに従う。障害の持ち越し先は障害で、取りやめの状態が無いため、送り直した `edit_blocker`・`resolve_blocker` は通常の決まりに従う。
  - 判定の順：対象の5つの操作のどれでも、3.2 のとおり `storage_unavailable` の次、`user_editing` の前に確かめる。
  - 読み取り：`get_board`・`open_board` の各項目と、`search` の結果の項目には、持ち越し先の項目の id `carried_to_id` と日付 `carried_to_date` を含める（持ち越し済みでなければどちらも `null`。4.2）。エージェントは、前の日のボードを読んだときに、持ち越し済みの項目を操作の対象から外せる。
  - 持ち越し先の項目が削除され、たどっても削除されていない項目に行き着かない場合（データモデル設計書 3章）、元の項目は持ち越し済みでなくなり、通常どおり受け付ける（その項目が残っている唯一の写しになるため）。
  - `agent revert` で持ち越し済みの項目を戻す場合の扱いは未決（9章）。
- A-01 のノート取得は、`get_board`（見出しと長さ）と `get_note`（本文）の組で満たす（4.2 参照）。
- `get_note` の扱い（2026-11-04 決定。コアの `domain::text_doc::get_note`）：
  - 見出しを指定しない（`--heading` なし）と、全文と版（`rev`）を返す。まだ書いていない日は空の本文と版 0（day の行は作らない）。
  - `--heading H`（MCP では `heading`）の `H` は、`get_board` の `headings` の1つ（例：`10:00 定例`）をそのまま渡す。照合は、`H` の前後の空白を除いた文字列と、見出しの文字（`## ` の後ろ。前後の空白を除いたもの。データモデル設計書 3章 note）の完全一致。時刻だけ（`10:00`）で件名つきの見出しには当たらず、`9:05` と `09:05` は別の見出し、件名の中の空白（全角を含む）も書いたとおりに比べる。`get_board` が返す文字をそのまま使えば必ず当たり、時刻の書き方を正規化する規則を増やさないため。件名だけでの照合や部分一致は設けない（当たる見出しが曖昧になるため）。
  - 同じ見出しが複数あるときは、当たった区間をすべて本文の順に返す。読み取りは何も変えないので、エラーにしない（`headings` に番号が無く、エージェントが区別する手段もないため）。見出しの無い区間（最初の見出しより前）は見出しでは選べない。全文は `--heading` を付けずに読む。時刻の付かない `## 良かったこと` は区間を区切らないので、直前の時刻付き見出しの区間に含まれる。
  - 区間の本文は、見出しの行から次の時刻付き見出しの前まで（末尾の空行も含む）を LF でつないだもの。それぞれに `heading` と、見出しの行のノート全体での行番号 `start_line`（1始まり）を付ける。`text` の行は本文に保存してある行のとおり（CR は行の文字なので、CRLF のノートでは各行の末尾に CR が残る。`edit_note` の `--expect` に渡す行の本文と同じ形で、`text` を LF で分けた行をそのまま渡せる）。`text` の n 行目は全体の `start_line + n - 1` 行目で、`edit_note` の `--line` はノート全体の行番号なので、エージェントは区間だけを読んでも書き換える行を求められる。
  - 判定の順：`permission_denied`（`read`）、`invalid_argument`（`H` が前後の空白を除くと空、または途中に改行を含む）、`not_found`（当たる見出しがない。まだ書いていない日も）。読み取りなので `storage_unavailable` は返さない（4.3）。`not_found` の `message` は、見出しが無いことと、`get_board` の `headings` の文字をそのまま渡すことを伝える。
  - 応答は 4.2。CLI のテキスト出力は、全文なら本文、見出し指定なら当たった区間の `text` を順に出す（複数のときは見出しの文字と `start_line` が分かる区切りを付ける。形は CLI を作るときに決める）。

### 3.1 その他の CLI コマンド

| CLI | 内容 |
| --- | --- |
| `yunagi show [--note]` | アプリを前面に表示する（Wayland 環境でのショートカット割当にも使う。操作仕様書 1章「OS 全体の操作」、ビルド・配布手順書 4.1 の案内がこのコマンドを前提にしている。まだ作っていない） |
| `yunagi doctor` | 診断情報（版、OS、保存先、ローカル API の状態）を出力する。業務データの内容は含めない。ローカル API の状態は、アーキテクチャ設計書 4.4 の接続の試し方で1回だけ調べて（待たない）、次のどれかで示す：`connected`（つながり認証も通った）、`no_listener`（待ち受けが無く、ロックも取れた。アプリが動いていない場合のほか、動いていてもデータベースを開いていない「見つからない」「開けなかった」の状態を含む。ここからは区別できないので、「動いていない」とは言わない）、`starting_or_unresponsive`（待ち受けが無く、ロックを別のプロセスが持っている。アプリの起動中、または待ち受けが動いていない。直接モードの操作が長引いているだけのこともある）、`listener_busy`（待ち受けはあるが一時的につながらない。パイプが全部ふさがっている・待ち行列が満杯など。4.4 の試し直しの対象で、`doctor` は1秒だけ試し直してからこれを示す。ロックは取らない）、`auth_failed`・`version_mismatch`・`denied`・`socket_path_too_long`・`default_dir_mismatch`（4.3 の `api_error` の `reason` と同じ。`socket_path_too_long` は、ソケットのパスが長すぎて（UNIX 系）、ロックを別のプロセスが持っているとき。ロックが取れれば `no_listener`。`default_dir_mismatch` は、待ち受けが無く、別の候補の既定ディレクトリを使っているプロセスがいて、直接モードに入れない場合。アーキテクチャ設計書 4.5）。取ったロックはすぐ離す。あわせて、自分の既定ディレクトリ（パスと、求めた環境変数の名前。値は出さない）と、別の候補の既定ディレクトリの状態（`absent`（無い）・`has_data`（データだけある）・`in_use`（使っているプロセスがいる。自分の保存先に別のデータベースがあり、環境を分けている形のときは `in_use_separate`）・`unknown`（調べられない）。アーキテクチャ設計書 4.5）を、ローカル API の状態とは別の欄で出す（ローカル API の状態は、環境を分けている形のときは `no_listener` のまま）。状態を決める順：自分の既定ディレクトリのロックを別のプロセスが持っていれば `starting_or_unresponsive`（ソケットのパスが長すぎるときは `socket_path_too_long`。ずれは調べない）、ロックが取れて、アーキテクチャ設計書 4.5 の判定 2 に当たれば `default_dir_mismatch`、それ以外は `no_listener`。ずれの手がかりを、業務データを含めずに利用者へ示すため |
| `yunagi uninstall [--purge]` | アプリと CLI を削除する。`--purge` でデータも削除 |
| `yunagi --version` | 版を表示する |

### 3.2 項目への書き込みの判定の順と、何も変えない成功（2026-11-03 決定）

3章の `edit_todo`・`edit_blocker`・`complete_todo`・`reopen_todo`・`resolve_blocker`（ID を指定する項目への書き込み）に共通の決まり。各操作の節には、その操作だけの決まりを書く。コアの実装は `domain::item`。

- 判定の順：次の順に確かめ、最初に当たったものを返す。どれにも当たらなければ今の値と比べる。
  1. `permission_denied`：送られた操作のスコープ（2章）。項目を見る前に確かめる（すべての操作に共通）。登録されていないエージェントには、何も許可されていないものとして扱う。権限はコアが確かめるので、アプリ経由（ローカル API）でも直接モードでも同じになる（アーキテクチャ設計書 4章 5）。
  2. `not_found`：ID の項目が無いか、削除されている。
  3. `invalid_argument`：はじめに取りやめた TODO（`close_reason` を含める）、次に種類の違う ID（`item_kind`・`use_operation` を含める）、続けて本文・解消メモの誤り（空・途中の改行）。前の2つは、引数を直しても時間をおいても成功しないので、最初に返す。
  4. `storage_unavailable`：画面の保存の書き込みに失敗して再試行している間（アーキテクチャ設計書 4.1）。
  5. `carried_over`：持ち越し済みの項目（3章）。
  6. `user_editing`：その操作が書く、利用者が入力する欄（`edit_todo`・`edit_blocker` は本文、`--note` 付きの `resolve_blocker` は解消メモ）を入力中のとき。状態だけを変える操作（`complete_todo`・`reopen_todo`、`--note` の無い `resolve_blocker`）は返さない。
  7. `note_exists`：`resolve_blocker` だけ（3章）。
- この順のため、今と同じ本文やすでに目的の状態でも、取りやめた TODO と種類の違う ID なら `invalid_argument`、保存の再試行中は `storage_unavailable`、持ち越し済みの項目なら `carried_over`、入力中なら `user_editing` になる。
- 何も変えない成功：今の値と比べて、送られた欄（状態・完了時刻・本文・解消メモ）がすべて同じなら、エラーにせず、何も書かずに成功を返す。エージェントは応答が届かなかったときなどに、同じ操作をそのままやり直してよい。エラーにすると、やり直したエージェントが、1回目が届いていたのか失敗したのかを区別できないため。一部の欄だけが違えば、違う欄だけを書く。
- 応答と記録：成功の応答には、実際に何かを変えたかを示す `updated`（`true` / `false`）を含める。CLI では、`--json` のときに同じ `updated` を含む JSON を出し、テキスト出力は各操作の節の文を表示する。終了コードはどれも 0。何も変えなかった場合（`updated: false`）は、change_log に記録せず（7章）、item の `updated_at` も変えない（データモデル設計書 3章）。記録するときは、実際に変えた欄だけを `before` / `after` に入れる。
- `add_todo`・`add_blocker` は項目を見ないので、`permission_denied`・`invalid_argument`（本文）・`storage_unavailable` の順に確かめ、その日の末尾に足す。記録は項目全体を `after` に入れる（データモデル設計書 change_log）。

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
| 7 | データを使えない（ロックが取れない） | `busy` |
| 8 | ローカル API を使えない（別の既定ディレクトリのアプリに届かない場合を含む） | `api_error` |

- 終了コード 7（`busy`）は、何も反映していないので、そのまま送り直してよい。終了コード 8（`api_error`）は、`reason`（4.3）で送り直してよいかが変わる（`result_unknown` の書き込みは反映されたか分からないので送り直さない）。テキスト出力でも `reason` を表示するので、`--json` なしで見分けられる。終了コードで再試行する道具は、7 だけを再試行の対象にする。
- `TEXT` に `-` を指定すると標準入力から読む（例：`git log -1 --format=%s | yunagi reflection append -`）。

### 4.2 MCP ツールの返り値（例）

`get_board`：

```json
{
  "date": "2026-11-02",
  "todos": [
    {"id": "01J...", "text": "API のレート制限を調査", "status": "open", "close_reason": null, "carry_count": 1, "origin": "human", "external_ref": null, "carried_to_id": null, "carried_to_date": null}
  ],
  "blockers": [
    {"id": "01J...", "text": "ステージング環境の権限待ち", "status": "open", "carry_count": 0, "origin": "note", "external_ref": null, "resolution_note": null, "carried_to_id": null, "carried_to_date": null}
  ],
  "reflection": "…",
  "note": {"headings": ["10:00 定例", "14:00 設計レビュー"], "length": 2380}
}
```

ノート本文はサイズが大きくなりうるため、`get_board` では見出しと長さのみを返し、本文は `get_note`（read）で見出し単位または全文を取得する。

`get_note`（2026-11-04 決定。3章）：

```json
{"date": "2026-11-02", "rev": 3, "body": "…"}
{"date": "2026-11-02", "rev": 3, "sections": [{"heading": "10:00 定例", "start_line": 12, "text": "## 10:00 定例\n議事…"}]}
```

`--heading` なしは1つ目（`body`）、ありは2つ目（`sections`。当たった区間を本文の順に。`rev` は読んだ時点のノートの版）。

返り値の細部（2026-11-03 決定。コアの `domain::board`）：

- `todos`・`blockers` は、その日の削除されていない項目を、画面と同じ並び順で返す。`todos` には取りやめた TODO（`status: "dropped"`）も入れ、理由を `close_reason`（`dropped` / `split` / `returned_external`。それ以外の TODO は `null`）で返す（取りやめた TODO への書き込みのエラーと同じ値。3章）。エージェントが、取りやめた TODO を操作の対象から外し、利用者に理由を正しく伝えられるようにするため。障害の項目には `close_reason` の欄を入れない（障害は取りやめず、状態は `open` / `resolved` だけなので。データモデル設計書 3章 item）。障害には代わりに `resolution_note` を入れる（無ければ `null`。未解消に戻した障害にも残っていることがある。3章）。逆に TODO には `resolution_note` の欄を入れない（2026-11-03 決定）。
- `note.length` は本文の文字数（Unicode のコードポイントの数。バイト数ではない）。`headings` は時刻付き見出しの文字（データモデル設計書 3章 note）。まだ書いていない日は `reflection` が `""`、`note` が `{"headings": [], "length": 0}`。
- `open_board` は `date`・`todos`・`blockers` だけを返す（振り返りとノートは返さない）。項目の形は `get_board` と同じで、`status` が `open` の項目だけを入れる。過去の日の項目は持ち越しても `status` を変えないので、持ち越し済みの項目も入る（`carried_to_id` で見分ける。件数の「残り」と同じく、その日の終わりの状態を表す）。
- どちらも読むだけで、day の行を作らない。

### 4.3 エラー

```json
{"error": {"code": "permission_denied", "required_scope": "todo", "message": "This agent is not allowed to modify TODOs. Enable it in Yunagi > Settings > Agents."}}
{"error": {"code": "conflict", "reason": "user_editing", "message": "The user is editing this item. Try again later."}}
```

エラーコード：`permission_denied`、`not_found`、`invalid_argument`、`conflict`、`storage_unavailable`、`busy`、`api_error`。`storage_unavailable` は、画面での保存の書き込みに失敗して再試行している間、対象に関係なくすべての書き込みに返す（読み取りは受け付ける。`permission_denied`・`not_found`・`invalid_argument` は先に返す。3.2 の判定の順、アーキテクチャ設計書 4.1）。直接モードで保存先にデータベースが見つからないときも `storage_unavailable` を返す（読み取りを含むすべての操作。アーキテクチャ設計書 6.3）。CLI・MCP の既定ディレクトリが無いときも同じコードで、`message` に探した既定ディレクトリと求めた環境変数の名前を書き、アプリを別の環境変数で起動していないか確かめるよう伝える（アーキテクチャ設計書 4.5）。

**接続の失敗（2026-11-04 決定。アーキテクチャ設計書 4.4）**：操作がコアに届く前、または届いたか分からないときに、CLI・MCP が返すエラー。どちらも項目の判定（3.2）には入らず、読み取りを含むすべての操作に返す。

```json
{"error": {"code": "busy", "message": "Yunagi is using its data and did not respond within 5 seconds. Nothing was changed. Try the same request again. If it keeps failing, ask the user to restart Yunagi."}}
{"error": {"code": "api_error", "reason": "result_unknown", "message": "The connection to Yunagi was lost after the request was sent. It may or may not have been applied. Read the data again before retrying."}}
```

- `busy`（終了コード 7）：ローカル API にもつながらず、ロックも 5 秒の間に取れなかった。または、アプリがデータベースを開き直している間（保存先の変更・復元）に届いた依頼を、5 秒で始められず、アプリが断った（アーキテクチャ設計書 4.4）。どちらも操作を何も反映していない。エージェントは同じ操作をそのままやり直してよい（追加の操作を含む）。続くときは、アプリの再起動を利用者に頼む。`storage_unavailable` と分けるのは、原因と次の動きが違うため（`storage_unavailable` は保存先の問題で、時間をおいても直らないことがある）。MCP では、ツールのエラーとして返すだけで、サーバーは終了せず、次の操作も受け付ける。
- `api_error`（終了コード 8。`reason` で分ける）：ローカル API の待ち受けはあるが、使えない。または、別の既定ディレクトリでアプリが動いていて、そちらに届かない（`default_dir_mismatch`）。
  - `auth_failed`：トークンが合わない（トークンのファイルが古いなど）。`message` で、アプリを再起動するよう伝える。
  - `version_mismatch`：アプリと CLI・MCP のプロトコルの版が合わない。`message` で、アプリと CLI の版をそろえるよう伝える。
  - `denied`：ソケット・パイプを開く権限が無い（`EACCES`）。別の利用者のアプリが同じ場所を使っているときなど。
  - `default_dir_mismatch`（2026-11-04 決定。アーキテクチャ設計書 4.5）：アプリと CLI・MCP の環境変数が違い、既定ディレクトリが別の場所になっている。ローカル API には届かず、別の候補の既定ディレクトリを使っているプロセス（ふつうはアプリ）がいて、直接モードで開くと利用者の画面と別のデータに書くか、同じデータベースを別のロックで開いてしまう場合に返す。`message` に、このコマンドの既定ディレクトリと、使っているプロセスのいる既定ディレクトリを書き、環境変数（`XDG_DATA_HOME` など）をアプリと同じにするよう伝える（例：`Yunagi is running with a different data folder (/b/yunagi) than this command uses (/a/yunagi). Nothing was changed. Ask the user to run the agent with the same environment variables as Yunagi (for example XDG_DATA_HOME). If Yunagi is not running, the same request may be retried.`）。環境をそろえるまで直らないので、エージェントは送り直さず、利用者に伝える（アプリが動いていないとき、別の直接モードの操作が原因のときだけ、そのまま送り直してよい。アーキテクチャ設計書 4.5）。待ち受けが応答しているわけではないので「待ち受けはあるが」という上の定義の例外で、直接モードへ切り替えないところは同じ。
  - `result_unknown`：依頼を送った後、応答の前に接続が切れた。書き込みは送り直さず、すぐこのエラーを返す。読み取りは、接続の手順（アーキテクチャ設計書 4.4）から1回だけ自動でやり直し（必ず1回）、やり直しでも切れたら同じエラーを返す。エージェントは、読み取りで反映されたかを確かめてから、必要なら利用者に伝える。
  - `socket_path_too_long`（2026-11-04 決定。アーキテクチャ設計書 4.4）：既定ディレクトリの下のソケットのパスが長すぎて（`sun_path` に収まらず。UNIX 系）、アプリが待ち受けられない。ロックが取れなければ（アプリが動いていれば）、5 秒待たずにすぐ返す。`message` は、既定ディレクトリが長すぎてローカル API を使えないことと、アプリを終了すれば同じ操作が使えること、環境変数（`XDG_DATA_HOME` など）で短い場所にすることを伝える。待っても直らず、`busy` の「再起動して」の案内では直らないため、送り直さず利用者に伝える。
  - どれも何も反映していない（`result_unknown` を除く）。直接モードへは切り替えない（`socket_path_too_long` は、ロックが取れて直接モードで処理できるなら、そちらで処理する。上のとおり、取れなかったときだけ返す）。

`conflict` には、エージェントが次の動きを決められるよう、理由を `reason` 欄で付ける。CLI の終了コードはどの理由でも 5 で、`--json` のときは同じ JSON を出し、テキスト出力では `message` を表示する。

`message` は、エラーコードと `reason` に合わせて書いた文にする。内部のエラーの文（Rust のエラーの Display。開発者向けで、ファイルのパスなどを含む）をそのまま使わない（2026-11-03 決定。書き方は README「エラーの表示（Rust）」）。

| `reason` | 意味 | エージェントの次の動き |
| --- | --- | --- |
| `changed` | 対象が直前に変更された（`edit_note` の `--expect` の不一致、`agent revert` の対象がその後に人に変更されている場合） | 最新の内容を読み直してから、操作を組み立て直す。`agent revert` は人が実行する操作で、差分を確かめたうえで `--force` で上書きもできる（7章） |
| `note_exists` | `resolve_blocker` の `--note` と異なる解消メモが、障害にすでにある（3章）。操作は何も反映していない | 最新の内容を読み直す。エージェントは既存の解消メモを書き換えられないため、足したい内容があれば利用者に伝える。解消だけが目的なら `--note` を付けずに送る（同じ `--note` のままやり直しても必ず失敗する） |
| `carried_over` | 対象の項目は持ち越し済みで、前の日に残っている元の項目である（3章）。操作は何も反映していない。エラーに `carried_to_id`・`carried_to_date` を含める | 同じ操作を `carried_to_id` の項目に送り直す（同じ ID のままでは、持ち越し先が削除されない限り成功しない）。送り直した先でも `carried_over` なら、同じようにたどる |
| `user_editing` | 利用者が対象を入力中（画面から知らされた守る行が古い版のもので、知らせ直しを待っている場合を含む。アーキテクチャ設計書 4.1）。操作は何も反映していない | 同じ操作を、しばらくしてからそのままやり直す |

`user_editing` の場合は、`message` でもその旨と、しばらくしてからやり直すよう伝える。文言は対象に合わせる（例：項目なら `The user is editing this item. Try again later.`、ノート・振り返りの行なら `The user is editing this line. Try again later.`）。`note_exists` の場合は、やり直しても成功しないことと次の動きを伝える（例：`This blocker already has a different resolution note. It was not changed. To resolve it, send the request without a note.`）。`carried_over` の場合は、持ち越し先へ送り直すことを伝える（例：`This item was carried over to 2026-11-03. It was not changed. Send the request to carried_to_id instead.`）。`message` の言語は未決（9章）。

### 4.4 ローカル API（アプリとの通信。2026-11-04 決定）

CLI・MCP サーバーがアプリに依頼する入口。通信路・トークン・待ち受けの作り方は、アーキテクチャ設計書 4.6。ここは、流す JSON と操作の中身。MCP のツールの返り値（4.2）と同じ形を返すので、MCP サーバーは返り値をそのまま MCP に載せられる。

**メッセージ**：JSON-RPC 2.0 を、1 行 1 メッセージの JSON（改行区切り）で送る。1 つの接続で `hello` → 依頼 1 つ → 応答 1 つを行い、閉じる。まとめての依頼（配列）と通知（`id` の無い依頼）は受け付けない。

```json
{"jsonrpc":"2.0","id":0,"method":"hello","params":{"protocol":1,"token":"<64 桁の 16 進>","client":"claude-code"}}
{"jsonrpc":"2.0","id":0,"result":{"protocol":1,"app_version":"0.0.0"}}
{"jsonrpc":"2.0","id":1,"method":"add_todo","params":{"text":"レート制限を調べる","date":"today"}}
{"jsonrpc":"2.0","id":1,"result":{"id":"01J...","date":"2026-11-04","text":"レート制限を調べる"}}
{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"…","data":{"error":{"code":"permission_denied","required_scope":"todo","message":"…"}}}}
```

- `client` は識別子（`yunagi mcp --client claude-code` の `claude-code`。CLI は `cli`）。省略すると `unknown`（2章）。初めての識別子は `read` だけの権限で登録される。使える形は、英小文字か数字で始まり、英小文字・数字・`-`・`_`・`.` だけの 64 文字まで。誤っていれば `-32602`。
- エラー：プロトコルの誤りは JSON-RPC の標準のコード（`-32700` JSON でない、`-32600` 依頼の形が誤り・`hello` より先に依頼を送った、`-32601` 知らない操作、`-32602` 引数が誤り・知らない引数名・必須の引数が無い）。4.3 のエラー（`permission_denied`・`not_found`・`invalid_argument`・`conflict`・`storage_unavailable`・`busy`・`api_error`）は、コード `-32000` で、`error.data.error` に 4.3 のオブジェクトをそのまま入れる。`hello` の失敗は `api_error` の `reason: auth_failed`（トークンが合わない）・`version_mismatch`（プロトコルの版が合わない。エラーにアプリの版 `server_protocol`）。
- 引数の名前は MCP のツールの引数にそろえる。省略できるのは `date`（既定は今日。`today`・`yesterday`・`YYYY-MM-DD`。形の誤りや存在しない日付は `invalid_argument`）。知らない名前の引数は、綴りの誤りを黙って無視しないよう `-32602`。

**操作**（名前は 3章のツール名）

| 操作 | 引数 | 成功の返り値 |
| --- | --- | --- |
| `get_board` | `date?` | 4.2 の `get_board` |
| `open_board` | `date?` | 4.2 の `open_board` |
| `get_note` | `date?`、`heading?` | 4.2 の `get_note` |
| `add_todo`・`add_blocker` | `text`、`date?` | `{id, date, text}`（追加した項目） |
| `edit_todo`・`edit_blocker` | `id`、`text` | `{updated, id, text}` |
| `complete_todo`・`reopen_todo` | `id` | `{updated, id, status}` |
| `resolve_blocker` | `id`、`note?` | `{updated, resolved, note_written, id, status, resolution_note}` |
| `append_note` | `text`、`date?`、`heading?` | `{updated, rev}` |
| `append_reflection` | `text`、`date?` | `{updated, rev}` |
| `edit_note` | `line`、`expected_text`、`replace`・`delete`（`true`）・`move_to` のうちちょうど 1 つ、`date?` | `{updated, rev}` |

- `append_note` の `heading` は、あれば現在時刻の見出しを入れてから追記する（CLI の `--heading [TITLE]`。`""` は件名なしの `## HH:MM`）。省略か `null` なら見出しを入れない。
- `edit_note` の `replace`・`delete`・`move_to` が 0 個か 2 個以上なら `-32602`。`line`・`move_to` は 0 以上の整数（0 の意味はコアが `invalid_argument` にする）。
- `updated` は 3.2 の意味（実際に何かを変えたか）。`updated: false` の成功では、画面への `changed` を出さない。
- `search`・`export_handoff` は、コアの入口を作るときに同じ形で足す。
- 権限・入力中・持ち越し・保存の失敗中の判定とエラーは、3章・3.2 のとおり（コアが行う。直接モードと同じ）。内部のエラーの文は応答に出さず（4.3）、読み書きの失敗は `storage_unavailable`（`message` は、何も変えていないことと、アプリを確かめるよう伝える）にする。
- データベースを使えるまで待つのは最大 5 秒。保存先の変更・復元の最中などで使えなければ、何も反映せずに `busy`（4.3）。アプリが止め始めていて始めなかった依頼も `busy`。

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

区間と障害の求め方の細部（2026-11-03 決定。コアの `domain::note_send::request_context`）：区間は送り元のノートの日（持ち越した TODO では元の日）の、いまの本文で求める。「同じ見出しの区間の行から送った障害」は、区間の行に送信済みの印を付けている障害（データモデル設計書 3章 note_send）で、行の順に並べ、同じ障害は1件にする。持ち越した障害は、最後の持ち越し先の項目の状態で未解消かを決め、その項目を載せる（持ち越し元の状態は持ち越した後に変わらないため）。コアは区間の行を空白だけの行も含めてそのまま返し、上の例のような行の整え方（見出しの付け方、空行の扱い）は書き出しを作るときに決める。

## 6. 導入（A-07）

`yunagi agent setup` は、検出したエージェントの設定に MCP サーバーを登録する。

| 対象 | 登録方法（案） |
| --- | --- |
| Claude Code | `claude mcp add` 相当の登録（コマンド：`yunagi mcp --client claude-code`） |
| Codex | Codex の設定ファイルに MCP サーバー定義を追記（コマンド：`yunagi mcp --client codex`） |

各エージェントの設定形式は変わりうるため、実装時に最新の公式ドキュメントで確認する。登録前に変更内容を表示し、利用者の確認を得てから書き込む。既存の設定はバックアップする。

**登録するコマンドに環境変数を含めるか（2026-11-04 決定）**：既定では含めない。`XDG_DATA_HOME` などの値を登録に書くと、既定ディレクトリの絶対パスを設定ファイルに書き残すことになり、既定ディレクトリごと移したとき（新しい端末への引っ越しなど）に古い場所を指すため（アーキテクチャ設計書 6章と同じ理由）。また、`agent setup` を実行した端末の値が、アプリを起動する環境の値と同じとは限らない。代わりに、`agent setup` は書き込む前に、アーキテクチャ設計書 4.5 の別の候補を調べ、ずれていれば（別の候補を使っているプロセスがいる、または自分の既定ディレクトリが無い）変更内容の表示に警告を添える：この端末で求めた既定ディレクトリと、別の場所でアプリが動いていること、アプリを起動する環境とエージェントを起動する環境で同じ `XDG_DATA_HOME` などを使うこと。警告が出ても、利用者が確認すれば登録できる。

例外：エージェントが MCP サーバーに、自分の環境変数のうち既定ディレクトリの変数を引き継がない場合は、その変数がないと `yunagi mcp` が標準の場所を見て、アプリと違う場所になりうる（検出できない向き。アーキテクチャ設計書 4.5）。判定の条件は、YUN-7 で公式ドキュメントを確かめた結果、引き継がないと分かったエージェントであること（確かめていない・引き継ぐエージェントには含めない。確かめるまでは、どのエージェントにも含めない）。その場合に限り、`agent setup` は、実行時に設定されている `XDG_DATA_HOME`（Linux）・`LOCALAPPDATA`（Windows）の値だけを登録に含める（`HOME` は含めない。引き継がれないと `yunagi mcp` がそもそも動けず、ほかの登録も壊すため）。含める内容は確認の表示に出す。例外にしても上の理由（絶対パスが古くなる）は残るが、書き残すのは Yunagi の設定ファイルではなくエージェントの設定で、`agent setup` を再実行すれば更新できる。登録済みの値が現在の環境の値と違うときは、`agent setup` が変更内容の表示で知らせる。引き継ぎの有無は、Claude Code と Codex の公式ドキュメントで確かめる（YUN-7。完了の条件に書き足してある）。確かめるまでは含めない。

## 7. 変更履歴（A-08）

- todo・blocker・append・note_edit に該当する操作は、MCP・CLI のどちらからでも、すべて change_log に記録する（operation は 3章の命名規則に従う）。ただし、何も変えずに成功を返した操作（解消済みの障害への `resolve_blocker`、今と同じ本文の `edit_todo`・`edit_blocker` など。3章）は記録しない。
- 利用者が画面で項目の状態を変えてから保存されるまでの間に届いた、同じ状態にする `complete_todo`・`reopen_todo`・`resolve_blocker` は、受け付けて成功を返すが、利用者の保存が書き込まれた時点で利用者の変更として扱い、その操作の記録から状態と完了時刻の欄を外す（何も残らなければ記録を消す。アーキテクチャ設計書 4.1）。利用者の保存が先に届いた場合に記録されないのと同じ結果にするため。
- MCP・CLI から作成された項目には `origin = agent` と agent_client_id（CLI は `cli`）を記録し、画面で印を表示する。
- `yunagi agent revert` は取り消しを実行し、対象がその後に人によって変更されている場合は `conflict`（`reason: changed`。終了コード 5）を返す（`--force` で上書き）。項目では、その記録の操作が変えた欄だけで判定し、取り消しもその欄だけを戻す（ほかの欄に人が加えた変更は残る。データモデル設計書 change_log）。取り消しの確定には 2章の確認が必要。

## 8. 第2フェーズで追加する操作（案。段階はロードマップ参照）

- エージェントの直接起動：`yunagi run ID --agent claude-code`
- フック連携：エージェントの作業終了時に要約を振り返りの下書きへ追記
- 変更の購読：`subscribe_changes`（MCP の通知機能を利用）

## 9. 未決事項

- エラーの `message` を利用者の言語設定（`locale`）に合わせて日英で出すか、英語固定にするか
- `agent revert` の対象が、記録の後に持ち越し済みになった項目である場合（持ち越す前にエージェントが本文を変えていた場合など）に、取り消しを拒むか、元の項目だけを戻すか（3章では持ち越し済みの項目への書き込みを拒むが、取り消しは人が確認して行う操作のため、別に決める）。記録の後に取りやめた TODO（`dropped`）になった項目の `todo.edit`・`todo.complete`・`todo.reopen` を取り消す場合も、同じく別に決める（3章では取りやめた TODO の本文・状態への書き込みを拒む）
- `cli` クライアントで作成した項目を、人が CLI で直接作ったものとして `origin = human` にする手段（対話端末での実行時など）を設けるか
