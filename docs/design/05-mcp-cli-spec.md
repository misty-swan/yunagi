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
- 確認は、CLI（対話端末）かアプリの設定画面が行い、確認を得た後の処理はローカル API の管理の操作（4.4。`set_agent_scopes`・`revert_change`）に頼む。API の操作自体は確認を行わない：トークンを読める同じ利用者のプロセスは、ソケットに直接送れる（擬似端末と同じ種類の抜け道で、防御の境界ではない。アーキテクチャ設計書 4.6、SECURITY.md）。管理の操作は呼び出し元のスコープを確かめない（`read` を外したクライアントがいても、利用者は管理を使える。2026-11-04 決定。YUN-94）。動きは 7章。
- エージェントの識別：`yunagi agent setup` は、登録するコマンドを `yunagi mcp --client claude-code` のように識別子付きで書き込む。識別子のない接続は `unknown` クライアントとして `read` のみを与える。`unknown` には `read` より多くのスコープを足せない（名乗らないすべての接続が書けるようになってしまうため。`agent permissions` が `invalid_argument` で断る。2026-11-04 決定。YUN-94）。
- 振り返りの既存の文章を書き換える操作は、どのスコープでも提供しない。
- 許可されていない操作は、エラーコード `permission_denied` と必要なスコープ名を返す。
- 権限はコアが確かめる（アーキテクチャ設計書 4章 5）ので、アプリ経由（ローカル API）でも直接モードでも同じになる。書き込みだけでなく読み取りも同じで、いま作ってある `get_board`・`open_board`・`get_note` は、コアの関数が呼び出し元のクライアントを受け取り、はじめに `read` を確かめる（2026-11-04 決定。コアの `domain::board`・`domain::text_doc`）。画面が使う読み取り（`get_screen_board`、`text_doc::get`）はエージェントの操作ではないので確かめない。登録されていないクライアントには何も許可しない（`read` も含む）。`search` の入り口（コアの `domain::search::search`）も同じ形（呼び出し元を受け取り、はじめに `read` を確かめる。2026-11-04 決定、YUN-90 で作った）。`export_handoff` の入り口（コアの `domain::handoff::export`。5章）も同じ形で、呼び出し元を受け取り、はじめに `read` を確かめる（2026-11-05 決定、YUN-70 で作った）。画面の書き出し（Mod+Shift+A。`handoff::build`）はエージェントの操作ではないので確かめない。

## 3. 操作一覧（MVP：A-01〜A-08）

命名規則：MCP ツール名 `<動詞>_<名詞>` ＝ CLI `yunagi <名詞> <動詞>` ＝ change_log の operation `<名詞>.<動詞>`。例：`complete_todo` ＝ `yunagi todo complete` ＝ `todo.complete`。CLI には短い別名を用意してよい（別名の列）。検索の `search` は名詞・動詞に分けない例外とする。

| 要件 | MCP ツール名 | CLI | CLI の別名 | スコープ |
| --- | --- | --- | --- | --- |
| A-01 | `get_board` | `yunagi board get [--date D]` | `yunagi board` | read |
| A-01 | `open_board` | `yunagi board open [--date D]`（未完了 TODO と未解消の障害のみ） | — | read |
| A-01 | `get_note` | `yunagi note get [--date D] [--heading H]` | `yunagi note show` | read |
| A-02 | `search` | `yunagi search QUERY [--from D] [--to D] [--kind K]... [--limit N]` | — | read |
| A-03 | `add_todo` | `yunagi todo add TEXT [--date D]` | — | todo |
| A-03 | `edit_todo` | `yunagi todo edit ID TEXT` | — | todo |
| A-03 | `complete_todo` / `reopen_todo` | `yunagi todo complete ID` / `yunagi todo reopen ID` | `yunagi todo done ID` | todo |
| A-04 | `add_blocker` | `yunagi blocker add TEXT [--date D]` | — | blocker |
| A-04 | `edit_blocker` | `yunagi blocker edit ID TEXT` | — | blocker |
| A-04 | `resolve_blocker` | `yunagi blocker resolve ID [--note TEXT]` | — | blocker |
| A-05 | `append_reflection` | `yunagi reflection append TEXT [--date D]` | `yunagi reflect append` | append |
| A-05 | `append_note` | `yunagi note append TEXT [--date D] [--heading [TITLE]]` | — | append |
| A-05 | `edit_note` | `yunagi note edit --line N --expect TEXT (--replace TEXT \| --delete \| --move-to M) [--date D]` | — | note_edit |
| A-06 | `export_handoff` | `yunagi handoff export ID`（TODO の ID。動きは 5章） | `yunagi handoff ID` | read |
| A-07 | —（MCP には出さない） | `yunagi agent setup [--claude-code] [--codex]` | — | — |
| A-08 | —（MCP には出さない） | `yunagi agent permissions [CLIENT] [--allow S]... [--deny S]...` / `yunagi agent history [--client NAME] [--unseen] [--limit N] [--before ID]` / `yunagi agent revert CHANGE_ID [--force]` | — | 閲覧は不要。変更・取り消しは 2章の確認が必要。動きは 7章 |

- `D` は `YYYY-MM-DD`、`today`、`yesterday` を受け付ける。省略時は論理日付の今日。`--date` を持たない操作（ID を指定する操作）は、ID の項目が属する日を対象にする。
- 要件 A-03 の「取り消し」は、完了の取り消し（未完了に戻す＝`reopen_todo`）を指す。エージェントの変更の取り消し（A-08 の `agent revert`）とは別の操作である。
- `--kind K` は `todo` / `blocker` / `note` / `reflection` を受け付ける（複数指定可。`--kind todo --kind note` のように繰り返す。MCP と API では引数 `kinds` の配列）。`--from`・`--to` は `D` と同じ形（その日を含む）。`--limit N`（MCP と API では `limit`）は返す件数の上限で、省略すると 50 件、200 件を超える指定は 200 件にする（0 は `invalid_argument`）。QUERY は前後の空白を除いた文字列全体の部分一致（検索の決まりはデータモデル設計書 4章）。
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
  - `agent revert` で持ち越し済みの項目の欄を戻す場合は、拒む（`carried_over`。2026-11-04 決定。7.3）。
- A-01 のノート取得は、`get_board`（見出しと長さ）と `get_note`（本文）の組で満たす（4.2 参照）。
- `get_note` の扱い（2026-11-04 決定。コアの `domain::text_doc::get_note`）：
  - 見出しを指定しない（`--heading` なし）と、全文と版（`rev`）を返す。まだ書いていない日は空の本文と版 0（day の行は作らない）。
  - `--heading H`（MCP では `heading`）の `H` は、`get_board` の `headings` の1つ（例：`10:00 定例`）をそのまま渡す。照合は、`H` の前後の空白を除いた文字列と、見出しの文字（`## ` の後ろ。前後の空白を除いたもの。データモデル設計書 3章 note）の完全一致。時刻だけ（`10:00`）で件名つきの見出しには当たらず、`9:05` と `09:05` は別の見出し、件名の中の空白（全角を含む）も書いたとおりに比べる。`get_board` が返す文字をそのまま使えば必ず当たり、時刻の書き方を正規化する規則を増やさないため。件名だけでの照合や部分一致は設けない（当たる見出しが曖昧になるため）。
  - 同じ見出しが複数あるときは、当たった区間をすべて本文の順に返す。読み取りは何も変えないので、エラーにしない（`headings` に番号が無く、エージェントが区別する手段もないため）。見出しの無い区間（最初の見出しより前）は見出しでは選べない。全文は `--heading` を付けずに読む。時刻の付かない `## 良かったこと` は区間を区切らないので、直前の時刻付き見出しの区間に含まれる。
  - 区間の本文は、見出しの行から次の時刻付き見出しの前まで（末尾の空行も含む）を LF でつないだもの。それぞれに `heading` と、見出しの行のノート全体での行番号 `start_line`（1始まり）を付ける。`text` の行は本文に保存してある行のとおり（CR は行の文字なので、CRLF のノートでは各行の末尾に CR が残る。`edit_note` の `--expect` に渡す行の本文と同じ形で、`text` を LF で分けた行をそのまま渡せる）。`text` の n 行目は全体の `start_line + n - 1` 行目で、`edit_note` の `--line` はノート全体の行番号なので、エージェントは区間だけを読んでも書き換える行を求められる。
  - 判定の順：`permission_denied`（`read`）、`invalid_argument`（`H` が前後の空白を除くと空、または途中に改行を含む）、`not_found`（当たる見出しがない。まだ書いていない日も）。読み取りは、画面の保存の失敗中を理由に `storage_unavailable` を返さない（4.3）。データベースの読み取り自体が失敗したときだけ `storage_unavailable`（4.3）。`not_found` の `message` は、見出しが無いことと、`get_board` の `headings` の文字をそのまま渡すことを伝える。
  - 応答は 4.2。CLI のテキスト出力は、全文なら本文、見出し指定なら当たった区間の `text` を順に出す（複数のときは、各区間の前に `-- 10:00 定例 (line 12) --` の区切りの行を付ける。区間が 1 つのときは付けない。2026-11-04 決定。YUN-90）。

### 3.1 その他の CLI コマンド

| CLI | 内容 |
| --- | --- |
| `yunagi show [--note]` | アプリの窓を前面に出す（最小化を解き、隠れていれば表示し、フォーカスを移す）。動いていなければ起動する。`--note` はノートモードで出す。Wayland 環境でのショートカット割当にも使う（操作仕様書 1章「OS 全体の操作」、ビルド・配布手順書 4.1、利用者ガイド 2章）。ローカル API だけを使い、直接モードは無い。動きは 3.3 |
| `yunagi doctor` | 診断情報（版、OS、保存先、ローカル API の状態）を出力する。業務データの内容は含めない。ローカル API の状態は、アーキテクチャ設計書 4.4 の接続の試し方で1回だけ調べて（待たない）、次のどれかで示す：`connected`（つながり認証も通った）、`no_listener`（待ち受けが無く、ロックも取れた。アプリが動いていない場合のほか、動いていてもデータベースを開いていない「見つからない」「開けなかった」の状態を含む。ここからは区別できないので、「動いていない」とは言わない）、`starting_or_unresponsive`（待ち受けが無く、ロックを別のプロセスが持っている。アプリの起動中、または待ち受けが動いていない。直接モードの操作が長引いているだけのこともある）、`listener_busy`（待ち受けはあるが一時的につながらない。パイプが全部ふさがっている・待ち行列が満杯など。4.4 の試し直しの対象で、`doctor` は1秒だけ試し直してからこれを示す。ロックは取らない）、`auth_failed`・`version_mismatch`・`denied`・`socket_path_too_long`・`default_dir_mismatch`（4.3 の `api_error` の `reason` と同じ。`socket_path_too_long` は、ソケットのパスが長すぎて（UNIX 系）、ロックを別のプロセスが持っているとき。ロックが取れれば `no_listener`。`default_dir_mismatch` は、待ち受けが無く、別の候補の既定ディレクトリを使っているプロセスがいて、直接モードに入れない場合。アーキテクチャ設計書 4.5）。取ったロックはすぐ離す。あわせて、自分の既定ディレクトリ（パスと、求めた環境変数の名前。値は出さない）と、別の候補の既定ディレクトリの状態（`absent`（無い）・`has_data`（データだけある）・`in_use`（使っているプロセスがいる。自分の保存先に別のデータベースがあり、環境を分けている形のときは `in_use_separate`）・`unknown`（調べられない）。アーキテクチャ設計書 4.5）を、ローカル API の状態とは別の欄で出す（ローカル API の状態は、環境を分けている形のときは `no_listener` のまま）。状態を決める順：自分の既定ディレクトリのロックを別のプロセスが持っていれば `starting_or_unresponsive`（ソケットのパスが長すぎるときは `socket_path_too_long`。ずれは調べない）、ロックが取れて、アーキテクチャ設計書 4.5 の判定 2 に当たれば `default_dir_mismatch`、それ以外は `no_listener`。ずれの手がかりを、業務データを含めずに利用者へ示すため。形（2026-11-04 決定。YUN-90。`--json` はこの JSON、テキストは `key: value` の行）：`{"version", "os", "arch", "protocol", "default_dir": {"path", "env_vars", "exists"}, "data_dir": {"path", "database_exists"} または null, "local_api": {"status", "app_version"（`connected` のときだけ）}, "other_default_dir": {"path", "status"} または null}`。上の 9 つに当たらない失敗は次のように示す：接続は受けたが認証に答えない（1 秒試し直しても）は `starting_or_unresponsive`、通信の形が誤っている・そのほかの入出力の失敗も `starting_or_unresponsive`。自分のロックファイルを権限などで調べられないときは `local_api.status` を `unknown` にする（待ち受けが無いとは言えないため。ロックファイルは作らずに調べ、取ったロックはすぐ離す）。環境変数が無くて既定ディレクトリを求められないときは、`default_dir` に `path: null` と `error` を入れ、`local_api` は `null`（調べられない）。終了コードは、診断できた場合はどの状態でも 0 |
| `yunagi mcp [--client ID]` | MCP サーバー（stdio）を起動する。`agent setup`（6章）が `--client claude-code` などの識別子つきで登録する。入出力は 4.5 |
| `yunagi agent setup [--claude-code] [--codex]` | Claude Code・Codex の設定に MCP サーバーを登録する（A-07）。変更内容を表示し、対話端末（TTY）での確認を得てから書く。端末でなければ何も書かず `permission_denied`（終了コード 3）。動きは 6章 |
| `yunagi agent permissions [CLIENT] [--allow S]... [--deny S]...` | エージェントごとの権限を見る。`--allow`・`--deny` があれば変える（A-08）。変える場合は、変更内容を表示し、対話端末（TTY）での確認を得てから書く。端末でなければ何も書かず `permission_denied`（終了コード 3）。動きは 7.1 |
| `yunagi agent history [--client NAME] [--unseen] [--limit N] [--before ID]` | エージェントの変更の一覧を、新しい順に出す（A-08）。確認は要らない。動きは 7.2 |
| `yunagi agent revert CHANGE_ID [--force]` | エージェントの変更を 1 件取り消す（A-08）。取り消す内容を表示し、対話端末（TTY）での確認を得てから書く。端末でなければ何も書かず `permission_denied`（終了コード 3）。動きは 7.3 |
| `yunagi uninstall [--purge]` | アプリと CLI を削除する。`--purge` でデータと WebView のデータも削除する（消すものと順番はビルド・配布手順書 4.4、アーキテクチャ設計書 6.5。コードは `crates/cli/src/uninstall/`。2026-11-05 決定。YUN-99）。消すものを一覧にして `[y/N]` の確認を得てから消す（`agent revert` と同じ。エージェントもシェルから呼べ、データを消す操作を黙って確定させないため）。端末でなければ何も消さず `permission_denied`（終了コード 3）、`n` や空の答えは取りやめ（何も消さず 0）、入力が閉じていれば失敗（1）。`--json` は断る（`invalid_argument`）。何も消さずに止まるときの終了コード：アプリか MCP サーバーが動いている（`yunagi.lock` が 5 秒待っても取れない）は `busy`（7）、`--purge` で `config.json` が読めない・壊れているは `storage_unavailable`（6）、Homebrew の Cask で入れたもの・正規の置き場所の外（開発用のビルドなど）は `conflict`（5。`reason` は `homebrew`・`unmanaged_install`。案内の文に `brew` のコマンドか見つけたパスを入れる）、環境変数が無く場所を求められないは `not_found`（4）か `storage_unavailable`（6）。消し始めてから消せなかったものがある（`~/.local/bin/yunagi` が、この導入を指すリンクでなく手で置いたファイルだったときを含む）ときは、ほかを消し続け、最後に「消したもの」「消せなかったもの（理由つき）」を標準出力に一覧にして、終了コード 1。消せなかったものが無ければ 0。何も消すものが無ければ、確認せずに `Nothing to remove.` を出して 0 |
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
- 例外：CLI の標準入力の誤り（4.1）と、ID の形の誤りと日付の指定（`date`）の形の誤りは、引数を読む段階で `invalid_argument` にするため、項目や権限を見る前に返る（ローカル API の層も CLI の引数の読み取りも同じ。`permission_denied` より先になる。2026-11-04 決定）。形の誤りは、スコープに関係なく、直してやり直さなければならないため。
- この順のため、今と同じ本文やすでに目的の状態でも、取りやめた TODO と種類の違う ID なら `invalid_argument`、保存の再試行中は `storage_unavailable`、持ち越し済みの項目なら `carried_over`、入力中なら `user_editing` になる。
- 何も変えない成功：今の値と比べて、送られた欄（状態・完了時刻・本文・解消メモ）がすべて同じなら、エラーにせず、何も書かずに成功を返す。エージェントは応答が届かなかったときなどに、同じ操作をそのままやり直してよい。エラーにすると、やり直したエージェントが、1回目が届いていたのか失敗したのかを区別できないため。一部の欄だけが違えば、違う欄だけを書く。
- 応答と記録：成功の応答には、実際に何かを変えたかを示す `updated`（`true` / `false`）を含める。CLI では、`--json` のときに同じ `updated` を含む JSON を出し、テキスト出力は各操作の節の文を表示する。終了コードはどれも 0。何も変えなかった場合（`updated: false`）は、change_log に記録せず（7章）、item の `updated_at` も変えない（データモデル設計書 3章）。記録するときは、実際に変えた欄だけを `before` / `after` に入れる。
- `add_todo`・`add_blocker` は項目を見ないので、`permission_denied`・`invalid_argument`（本文）・`storage_unavailable` の順に確かめ、その日の末尾に足す。記録は項目全体を `after` に入れる（データモデル設計書 change_log）。

### 3.3 アプリを前面に出す（`yunagi show`。2026-11-04 決定。YUN-88）

デスクトップ環境のキーボード設定から呼ぶ使い方が主（Wayland では OS 全体のショートカットが効かないことがあるため。操作仕様書 1章「OS 全体の操作」、アーキテクチャ設計書 3.5）。呼び出し元の環境は、ふだんのシェルとは違い、`PATH` に `~/.local/bin` が入っていないことが多い（ビルド・配布手順書 4.1 の 7、利用者ガイド 2章）。

- **ローカル API だけを使い、直接モードは無い**。窓はアプリにしか無いので、アプリが動いていなければ、直接モードに切り替える代わりにアプリを起動する。接続の手順（アーキテクチャ設計書 4.4）は使わない（ロックも取らない）。依頼は、ローカル API の `show_app`（4.4）。データにもスコープにも触れない。
- 動き（操作ごと）：
  1. ローカル API に接続して認証し、`show_app` を頼む。つながれば、窓の操作の結果を待たずに（アプリは頼みを受け取った時点で）成功とする。
  2. 待ち受けが無ければ、アプリを起動する（1 回の実行で 1 回だけ）。起動した後は、手順 1 を 50 ミリ秒ごとに最大 30 秒（`show::START_WAIT`）試し直す。データベースを開くには、マイグレーションなどで数秒かかることがある。待ち受けはアプリがデータベースを開き終えてから始まる（アーキテクチャ設計書 4.4）ため、起動の直後は「待ち受けが無い」状態が続く。
  3. 待ち受けはあるが一時的につながらない（保存先の変更・復元で開き直している最中など。アーキテクチャ設計書 4.4 の手順 1）だけのときは、アプリがいるので起動せず、同じ手順の期限（5 秒）まで試し直す。それでもつながらなければ `busy`。
  4. 待ち受けはあるが使えない（認証の失敗・版の不一致・権限）は、起動せず `api_error`（4.3 の `auth_failed`・`version_mismatch`・`denied`）。アプリはそこにいるので、起動しても直らない。
- **アプリの起動**：この実行が終わっても動き続けるよう、標準入出力を閉じて切り離す（Unix は新しいプロセスグループ、Windows は `DETACHED_PROCESS`）。起動する場所は、環境変数 `YUNAGI_APP`（実行ファイル。macOS は `.app`）があればそれだけ（指す先が無い・実行ファイルでない（フォルダなど。macOS は `.app` のフォルダを除く）なら `app_not_found`。探し直さない）。`--note` のときは、アプリの起動の引数に `--note` を付ける（`open` では `--args --note`。引き渡しを受けるアプリが、ノートモードで出すため）。無ければ、この実行ファイルの実体（リンクを解いたもの）から探す（`show::locate`）：Linux は `../Yunagi.AppImage`（`~/.local/opt/yunagi/bin/yunagi` の隣。ビルド・配布手順書 4.1）、同じ場所の `yunagi-app`（開発のビルド）、`../yunagi-app`。macOS はこの実行ファイルを含む `.app` を `open` で起動する（無ければ同じ場所の `yunagi-app`）。Windows は `..\Yunagi.exe`、同じ場所の `yunagi-app.exe`。Linux の tar.gz・.deb の置き場所はまだ決めていない（配布の課題で決め、そのとき探す場所を足す）。見つからなければ何も起動せず `app_not_found`。
- **2 つ目のアプリを起動してしまわない**：データベースを見つけられない・開けない間の起動時の画面（アーキテクチャ設計書 4.4）は、アプリが動いていてもロックも待ち受けも持たないので、この実行からは「動いていない」ように見える。そのため、アプリ側で単一インスタンスにする（アーキテクチャ設計書 3.5）。起動した 2 つ目のプロセスは、動いているアプリに引き渡して成功で終わり、動いているアプリが窓を前面に出す（引数に `--note` があれば、ノートモードの頼みとして渡る。ノートモードにできる状態になったときに反映される。操作仕様書 1章）。この実行は、**直接起動したプロセスが、待ち受けが開く前に成功で終わった**ことを、引き渡しとみなして成功にする（待ち受けが開くのを待たない。待ち受けは、起動時の画面の間は開かないため）。macOS の `open` は、アプリの起動を待たずに成功で終わるので、そうはみなさず、待ち受けが開くまで待つ（起動時の画面の間に引き渡された場合は、期限（30 秒）で `start_timeout` になるが、窓は前面に出ている）。D-Bus が使えないなど、単一インスタンスにできない環境（Linux）では、2 つ目のアプリがそのまま起動し、ロックを待って利用者に知らせる（アーキテクチャ設計書 4章 4）。
- **ソケットのパスが長すぎる**（UNIX 系。アーキテクチャ設計書 4.4）：アプリは待ち受けられず、API では頼めない。この場合だけ、起動の引き渡し（上）が頼りになる。アプリを起動して、起動したプロセスが 2 秒以内に成功で終われば（動いているアプリに引き渡した）成功とし、まだ動いていれば（新しく起動したアプリ）成功とする。失敗で終われば `launch_failed`。`--note` は、起動の引数で渡る。macOS の `open` は、起動に失敗しても成功で終わることがあり、そのときは気づけず成功にする（`open` が失敗で終われば `launch_failed`）。
- 出力：成功は `Asked Yunagi to come to the front.`（起動したときは `Started Yunagi and asked it to come to the front.`）。`--json` は `{"requested": true, "launched": 起動したか}`（起動したプロセスが動いているアプリに引き渡して終わったときは、起動したのではないので偽で、出力は `Asked Yunagi to come to the front.`）。窓の操作は OS が制限することがある（Wayland のコンポジターが別のアプリからのフォーカスの移動を止めるなど。操作仕様書 1章）ので、`requested` は前面に出したことまでは約束しない。
- エラー：`app_not_found`・`launch_failed`・`start_timeout`（どれも `api_error`。終了コード 8。4.3）は `yunagi show` だけが返す。`busy`・`auth_failed`・`version_mismatch`・`denied`・`unexpected` は、4.3 と同じ意味で返す。引数の誤り（知らないオプション、`--note=1` など値の付いた `--note`、余計な引数）は `invalid_argument`（終了コード 2）。MCP には `show` の道具が無い（`tools/list` に無く、ツールとして呼ぶと `-32602`）。
- `--note` は `show` の後ろに書く（`blocker resolve` の `--note TEXT` は値を取るため、コマンドを読む前の引数の区切りで区別する）。ノートモードにする画面の決まりは、操作仕様書 1章「OS 全体の操作」。

## 4. 入出力

### 4.1 CLI

- 既定は人が読みやすいテキスト出力。`--json`（どこに書いてもよい）で機械可読な JSON を出力する。`--json` の成功は、4.2・4.4 の返り値をそのまま標準出力に 1 行で出し、失敗は 4.3 の `{"error": {...}}` を標準出力に出す（終了コードは同じ）。テキスト出力の失敗は、標準エラーに 1 行（`yunagi: <code> (<reason>): <message>`。`reason` と、`permission_denied` の必要なスコープも出す）。環境を分けている形の警告（アーキテクチャ設計書 4.5 の 3）は、どちらでも標準エラーに 1 行出し、標準出力は変えない。
- テキスト出力の形（2026-11-04 決定。YUN-90。元にするのは、アプリ経由でも直接モードでも同じ返り値の JSON なので、どちらの経路でも出力は同じ）：
  - `board get`・`board open`：`# 日付`、`## TODO`、`## Blockers`、（`get` だけ）`## Reflection`、`## Note (N characters)` と見出しの一覧。項目は `[ ] ID  本文`（完了・解消は `[x]`、取りやめた TODO は `[-]`）で、後ろに `(closed: 理由, carried 2x, carried over to 日付 as ID)` の注記を付け、障害の解消メモは次の行に `    -> メモ`。項目が無い節は `(none)`。
  - `note get`：全文は本文、見出し指定は区間の `text`（3章）。
  - `handoff export`：依頼文（5章）をそのまま出す（貼り付けやパイプで使うため。末尾は改行 1 つ）。`--json` は `{"id", "markdown"}`（4.2）。何も書かず、`--date` は受けつけない（ID の項目が属する日が対象）。
  - `search`：項目は `日付 種類 ID  本文`、ノート・振り返りは一致した行ごとに `日付 種類:行番号  行の本文`。0 件は `No results.`。
  - `doctor`：`key: value` の行（3.1）。
  - 書き込み（2026-11-04 決定。YUN-91）：成功は 1 行で、3章の文（`Resolved.`・`Note added.`・`Already resolved. Nothing changed.`・`Completed.`・`Already done. Nothing changed.`・`Reopened.`・`Already open. Nothing changed.`・`Edited.`・`Same text. Nothing changed.`）に次を足す。`add_todo`・`add_blocker` は `Added TODO ID (日付).`・`Added blocker ID (日付).`（あとで編集や完了に使う ID を出す）。`append_note`・`append_reflection` は `Appended.`、`edit_note` は `Edited.`、どちらも何も変えなかったとき（`updated: false`）は `Nothing changed.`。`resolve_blocker` が解消と解消メモの両方を行ったときは `Resolved. Note added.`。`--note` を付けずに解消した障害に前の解消メモが残っていたら、次の行に `Existing resolution note: 本文` を出す（3章。エージェントが利用者に伝えられるように）。
  - 書き込みの失敗のテキスト（標準エラー）は、`message` の後ろに、次の動きを決める欄を `[...]` で添える（`--json` では欄がそのまま出るため、テキストだけの補い）：`carried_over` は `[carried to: ID (日付)]`、`note_exists` は `[existing resolution note: 本文]`、種類の違う ID の `invalid_argument` は `[use yunagi blocker edit]` のように使う CLI のコマンド（無ければ `[ask the user]`）。必要なスコープは従来どおり `[required scope: S]`。
- 権限と変更履歴の出力（2026-11-04 決定。YUN-94。元にするのは 4.4 の管理の操作の JSON）：
  - `agent permissions`：`CLIENT  SCOPES  LAST SEEN` の表（スコープは 2章の表の順にコンマ区切り、無ければ `(none)`、最後の接続は端末の時刻で `YYYY-MM-DD HH:MM`、まだなら `-`）。`--json` は `{"agents": [...]}`（`CLIENT` があればその 1 件）。変える場合は 7.1。
  - `agent history`：1 件を 1 行で `* ID  日時  クライアント  operation  対象`（`*` は未確認のエージェントの変更。人の取り消しの記録のクライアントは `you`）、続く行に変えた内容を出す（項目の追加は本文、欄の変更は `欄: 前 -> 後`、ノート・振り返りは文字数の前後）。取り消し済みの記録には `[reverted]`、取り消しの記録には `(reverts ID)` を付ける。0 件は `No changes.`。続きがあれば最後に `(older changes: yunagi agent history --before ID)`。`--json` は `{"changes": [...], "next_before": ID または null}`。
  - `agent revert` の出力は 7.3。`--json` は、確認の質問を含む形（`agent permissions` の `--allow`・`--deny`、`agent revert`）には付けられない（`invalid_argument`。`agent setup` と同じ）。
- 書き込みの引数の決まり（2026-11-04 決定。YUN-91）：
  - `TEXT`（`todo add`・`todo edit` の本文、`blocker add`・`blocker edit`・`reflection append`・`note append` の本文）と `blocker resolve` の `--note` は、`-` を指定すると標準入力から読む（接続の前に読む）。`--expect` と `--replace` はノートの行そのものを渡すので、`-` も文字として渡し、標準入力からは読まない（`-` だけの行を扱えるように）。`-` を受けつける欄はどのコマンドでも 1 つだけなので、標準入力は 1 回しか読まない。標準入力の誤り（UTF-8 でない・大きすぎる）は、ID や日付の形の誤りと同じく接続の前に返るので、権限の誤りより先になる（3.2 の例外）。読んだ本文の複数行の可否はコアの決まりのままで、`reflection append`・`note append` は複数行を受けつけ（`git log` の出力など）、項目の本文と解消メモは 1 行だけ（途中の改行は `invalid_argument`。3章）。
  - 同じオプションを 2 回以上書くと `invalid_argument`（前の指定の誤りを、後ろの指定で隠さないため。繰り返せるのは `search` の `--kind` だけ。読み取りの `--date` も同じ）。`note append` で `--heading` の直後に `TEXT` を置くと件名として読まれ、`TEXT` が無くなるので、誤りの文でそう伝える。
  - `--delete` は値を取らない（`--delete=x` は `invalid_argument`）。`--move-to M`・`--line N` は 0 以上の整数（数でないものは `invalid_argument`。0 の意味はコアが決める）。`note edit` は `--line`・`--expect` と、`--replace`・`--delete`・`--move-to` のうちちょうど 1 つが要る（足りない・重なるは `invalid_argument`）。`--expect` の値は前後の空白を除かずにそのまま渡す。空の行は `--expect=` と書く。
  - `note append` の `--heading` は値を省略できる：次の引数が `-` で始まらなければ件名として読み、そうでなければ（または末尾なら）件名なしの `## HH:MM` にする。`-` で始まる件名と、件名なしで `--heading` の直後に `TEXT` を置く場合は `=` で書く（`--heading=-x`、`--heading=`）。読み取りの `note get --heading` は値が要る。
  - `--` で始まる値（例：`--expect ---`）は、読み取りの規則（下記）どおり `--name=VALUE` で書く。値が無いときの誤りの文にもそう書く。
  - ID を指定する操作（`todo edit`・`todo complete`・`todo reopen`・`blocker edit`・`blocker resolve`）は `--date` を受けつけない（`unknown option`）。`todo done` は `todo complete`、`reflect append` は `reflection append` の別名。
  - ID と日付の形の誤り、引数の過不足、標準入力の誤り（UTF-8 でない・約 0.7 MiB 超）は接続の前に `invalid_argument`。本文の誤り（空・途中の改行・`--line 0` など）は CLI で先に弾かず、コアの判定の順（3.2）に任せる（`permission_denied` と `not_found` が先になる）。
- 引数の誤りは `invalid_argument`（終了コード 2）で、`--json` なら同じ形式で出す（誤りがあっても引数を最後まで読んで `--json` の指定を決める）。オプションの値は `--name 値` か `--name=値` で、`--name` の次が `--` で始まる引数なら値が無いものとする（`--date --json` の `--json` を日付として読まない。`--` で始まる値は `--name=--x` と書く）。UTF-8 でない引数も `invalid_argument`。標準入力から読む `TEXT` の大きさの上限は約 0.7 MiB（JSON にしたときに依頼の上限 4 MiB に必ず収まる大きさ）で、超えると `invalid_argument`。ID・日付の形の誤りは、接続する前に返す（3.2）。
- CLI 自身の不具合（知らない操作を呼んだなど）は、コード `internal`（終了コード 1）で返す。エージェントが見分ける必要のある分類ではない（4.3 の表のコードは変えない）。
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

`search`（2026-11-04 決定。YUN-90。コアの `domain::search`）：

```json
{"query": "レート制限", "hits": [
  {"kind": "todo", "date": "2026-11-02", "id": "01J...", "text": "API のレート制限を調査", "carried_to_id": null, "carried_to_date": null},
  {"kind": "blocker", "date": "2026-11-01", "id": "01J...", "text": "…", "resolution_note": null, "carried_to_id": null, "carried_to_date": null},
  {"kind": "note", "date": "2026-11-02", "matches": [{"line": 14, "text": "外部APIの429が増えている。レート制限は…"}]},
  {"kind": "reflection", "date": "2026-11-01", "matches": [{"line": 2, "text": "…"}]}
]}
```

- `query` は前後の空白を除いた検索語。`hits` は検索の並び（日付の新しい順。同じ日は TODO・障害・ノート・振り返りの順）で、`limit` 件まで。項目の形は `get_board` の項目と同じ欄のうち `id`・`text`・（障害の）`resolution_note`・`carried_to_id`・`carried_to_date` を返す（`status` などは `get_board` で読む）。持ち越し済みでなければ `carried_to_id`・`carried_to_date` は `null`（3章の持ち越しの読み取り）。
- ノート・振り返りは、本文すべてを返すと大きいので、検索語を含む行（`matches`。比べ方は検索と同じ：3 文字以上は大文字・小文字を区別せず、2 文字以下は ASCII の英字だけ区別しない。上から 5 行まで）だけを返す。`line` は本文の行の番号（1 始まり。`edit_note` の `--line` と同じ数え方）で、`text` は行の本文。検索語が複数の行にまたがるとき、または検索の比べ方と文字の大小の扱いが一致しない文字（Unicode の特殊な大文字・小文字など）のときは `matches` が空になりうる（CLI のテキスト出力は `(no single matching line; read it with note get)` と出す）。全文は `get_note` で読む。
- 権限は `read`。結果が 0 件でも成功（`hits` は空）。

`export_handoff`（2026-11-05 決定。YUN-70。5章。コアの `domain::handoff`）：

```json
{"id": "01J...", "markdown": "## Task\nAPI のレート制限を調査\n\n## Context (from today's notes)\n- 10:00 定例: …\n\n(Exported from Yunagi on 2026-11-02)\n"}
```

- `id` は対象の TODO の ID、`markdown` は依頼文（5章の形。末尾は改行 1 つ）。権限は `read`。何も変えないので、同じ操作を何度送ってもよい。
- エラー（判定の順）：ID の形の誤りは引数を読む段階で `invalid_argument`（3.2 の例外と同じく、権限より先）。続けて `permission_denied`（`read`。項目を見る前）、`not_found`（ID の項目が無いか、削除されている）、`invalid_argument`（ID が障害を指す。3.2 の種類の違いと同じ形で `item_kind: "blocker"`・`use_operation: null`、`message` は書き出せるのは TODO だけと伝える）。完了した TODO・取りやめた TODO・持ち越し済みの TODO は、読み取りだけなので断らずに書き出す（`carried_over` は返さない）。画面の保存の失敗中でも書き出せる（読み取りは `storage_unavailable` を返さない。4.3）。

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

`permission_denied` のうち、スコープの不足ではなく、対話端末（TTY）での確認が要る操作を端末でない実行で呼んだもの（2章。`agent setup`、`agent permissions` の `--allow`・`--deny`、`agent revert`）には、`required_scope` を付けず、`message` に端末で実行し直すよう書く（例：`Confirmation needs a terminal. Nothing was changed. Run `yunagi agent revert` yourself in a terminal, or use the Yunagi app (Settings > Agents).`）。終了コードは同じ 3（2026-11-04 決定。YUN-93。`agent permissions`・`agent revert` は YUN-94）。`agent revert` が返す `conflict` の理由は、`changed`（`changed_fields` つき）・`carried_over`（`carried_to_id`・`carried_to_date` つき）・`user_editing` で、取りやめた TODO は `invalid_argument`（`close_reason` つき）。7.3。

エラーコード：`permission_denied`、`not_found`、`invalid_argument`、`conflict`、`storage_unavailable`、`busy`、`api_error`。`storage_unavailable` は、画面での保存の書き込みに失敗して再試行している間、対象に関係なくすべての書き込みに返す（読み取りは受け付ける。`permission_denied`・`not_found`・`invalid_argument` は先に返す。3.2 の判定の順、アーキテクチャ設計書 4.1）。直接モードで保存先にデータベースが見つからないときも `storage_unavailable` を返す（読み取りを含むすべての操作。アーキテクチャ設計書 6.3）。データベースの読み書きそのものが失敗したとき（ディスクのエラー、壊れたデータベースなど。2026-11-04 決定）も、読み取りを含めて同じコードで返す。`message` は何も変えていないこととアプリを確かめるよう伝えるだけにし、内部の文は含めない（ログに残す）。読み取りが断られるのは、この読み書きの失敗のときだけである。CLI・MCP の既定ディレクトリが無いときも同じコードで、`message` に探した既定ディレクトリと求めた環境変数の名前を書き、アプリを別の環境変数で起動していないか確かめるよう伝える（アーキテクチャ設計書 4.5）。

**接続の失敗（2026-11-04 決定。アーキテクチャ設計書 4.4）**：操作がコアに届く前、または届いたか分からないときに、CLI・MCP が返すエラー。どちらも項目の判定（3.2）には入らず、読み取りを含むすべての操作に返す。

```json
{"error": {"code": "busy", "message": "Yunagi is using its data and did not respond in time. Nothing was changed. Try the same request again. If it keeps failing, ask the user to restart Yunagi."}}
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
  - `app_not_found`・`launch_failed`・`start_timeout`（2026-11-04 決定。YUN-88。`yunagi show` だけが返す。3.3）：`app_not_found` はアプリが動いておらず、起動するアプリも見つからない（`message` で、利用者に起動してもらうか、環境変数 `YUNAGI_APP` にアプリの場所を設定するよう伝える）。`launch_failed` は起動できなかった（実行できない、すぐ異常終了した）。`start_timeout` は起動したが、30 秒たっても待ち受けが開かなかった（`message` で、アプリがまだ起動中か、利用者の操作（保存先の選択など）を待っているかもしれないので、窓を探すよう伝える）。どれも何も変えていない。
  - `unexpected`（2026-11-04 決定。YUN-90）：待ち受けとの通信が、上のどれにも当たらない形で失敗した（応答の形が誤っている、`hello` を断られた理由が分からない）。`message` は、何も変えていないことと、アプリの再起動（またはアプリと `yunagi` の版をそろえること）を利用者に頼むよう伝える。内部の文は標準エラーに残す。
  - どれも何も反映していない（`result_unknown` を除く）。直接モードへは切り替えない（`socket_path_too_long` は、ロックが取れて直接モードで処理できるなら、そちらで処理する。上のとおり、取れなかったときだけ返す）。

`conflict` には、エージェントが次の動きを決められるよう、理由を `reason` 欄で付ける。CLI の終了コードはどの理由でも 5 で、`--json` のときは同じ JSON を出し、テキスト出力では `message` を表示する。

`message` は、エラーコードと `reason` に合わせて書いた文にする。内部のエラーの文（Rust のエラーの Display。開発者向けで、ファイルのパスなどを含む）をそのまま使わない（2026-11-03 決定。書き方は README「エラーの表示（Rust）」）。

| `reason` | 意味 | エージェントの次の動き |
| --- | --- | --- |
| `changed` | 対象が直前に変更された（`edit_note` の `--expect` の不一致、`agent revert` の対象がその後に人に変更されている場合） | 最新の内容を読み直してから、操作を組み立て直す。`agent revert` は人が実行する操作で、差分を確かめたうえで `--force` で上書きもできる（7.3）。`agent revert` のエラーには、食い違う欄 `changed_fields` を含める |
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

- `client` は識別子（`yunagi mcp --client claude-code` の `claude-code`。CLI は `cli`）。省略すると `unknown`（2章）。初めての識別子は `read` だけの権限で登録される（このとき画面に `changed` を出す。設定画面のエージェントの一覧が変わるため）。登録できる識別子は、固定の `cli`・`unknown` を含めて 64 までで、超えると新しい識別子の依頼は `invalid_argument`（登録の整理を利用者に頼む。整理の手段は 7.4）。使える形は、英小文字か数字で始まり、英小文字・数字・`-`・`_`・`.` だけの 64 文字まで。誤っていれば `-32602`。`params` がオブジェクトでないときも `-32602`。
- エラー：プロトコルの誤りは JSON-RPC の標準のコード（`-32700` JSON でない、`-32600` 依頼の形が誤り・`hello` より先に依頼を送った、`-32601` 知らない操作、`-32602` 引数が誤り・知らない引数名・必須の引数が無い）。4.3 のエラー（`permission_denied`・`not_found`・`invalid_argument`・`conflict`・`storage_unavailable`・`busy`・`api_error`）は、コード `-32000` で、`error.data.error` に 4.3 のオブジェクトをそのまま入れる。`hello` の失敗は `api_error` の `reason: auth_failed`（トークンが合わない）・`version_mismatch`（プロトコルの版が合わない。エラーにアプリの版 `server_protocol`）。
- 引数の名前は MCP のツールの引数にそろえる。省略できるのは `date`（既定は今日。`today`・`yesterday`・`YYYY-MM-DD`。形の誤りや存在しない日付は `invalid_argument`）。`search` の `from`・`to` も同じ形で、省略すると期間の制限なし。知らない名前の引数は、綴りの誤りを黙って無視しないよう `-32602`。

**操作**（名前は 3章のツール名）

| 操作 | 引数 | 成功の返り値 |
| --- | --- | --- |
| `get_board` | `date?` | 4.2 の `get_board` |
| `open_board` | `date?` | 4.2 の `open_board` |
| `get_note` | `date?`、`heading?` | 4.2 の `get_note` |
| `search` | `query`、`from?`、`to?`、`kinds?`（種類の配列）、`limit?` | 4.2 の `search` |
| `add_todo`・`add_blocker` | `text`、`date?` | `{id, date, text}`（追加した項目） |
| `edit_todo`・`edit_blocker` | `id`、`text` | `{updated, id, text}` |
| `complete_todo`・`reopen_todo` | `id` | `{updated, id, status}` |
| `resolve_blocker` | `id`、`note?` | `{updated, resolved, note_written, id, status, resolution_note}` |
| `append_note` | `text`、`date?`、`heading?` | `{updated, rev}` |
| `append_reflection` | `text`、`date?` | `{updated, rev}` |
| `edit_note` | `line`、`expected_text`、`replace`・`delete`（`true`）・`move_to` のうちちょうど 1 つ、`date?` | `{updated, rev}` |
| `export_handoff` | `id`（TODO の ID） | `{id, markdown}`（4.2。読み取り） |

**窓の操作**（`show_app`。2026-11-04 決定。YUN-88）：`yunagi show`（3.3）が頼む。データの操作ではないので、上の表にも管理の操作にも入れず、MCP には出さない。データベースの錠も、呼び出し元のクライアントの登録（`agent_client`）も、スコープの判定も使わない（トークンの認証だけ）。そのため、保存先の変更・復元で開き直している最中でも、5 秒待たずに効き、画面にデータの変更（`changed`）も知らせない。

| 操作 | 引数 | 成功の返り値 |
| --- | --- | --- |
| `show_app` | `note?`（真偽。省略は偽） | `{requested: true}` |

- 窓を前面に出す部品は、OS 全体のショートカット（操作仕様書 1章）と同じ（`global_shortcut::bring_to_front`）。頼みを受けたアプリは、窓を前面に出して画面にイベント `show-requested`（`{note}`）を知らせる。`note` が真のとき、画面がノートモードにする（決まりは操作仕様書 1章）。窓の操作の失敗（窓が無い、OS が拒む）は、応答に出さずアプリのログに出すだけで、`requested: true` を返す。
- 引数の名前が知らないもの、`note` が真偽でないものは `-32602`。

**管理の操作**（A-08。2026-11-04 決定。YUN-94）：利用者が行う管理で、MCP には出さない（`tools/list` に無く、ツールとして呼ぶと `-32602`）。CLI の `agent permissions`・`agent history`・`agent revert` が使う。アプリの設定画面・変更履歴 S-06 は、この API を通さず、同じコアを Tauri コマンドで呼ぶ（7.4）。呼び出し元のスコープは確かめない（2章）。確認はこの操作の外（CLI の端末か画面）で行う。画面の入力中・保存の失敗中は、取り消しだけが見る（7.3）。コアは `domain::agent_admin`・`domain::history`。

| 操作 | 引数 | 成功の返り値 |
| --- | --- | --- |
| `list_agents` | なし | `{agents: [{id, name, scopes, created_at, last_seen_at}]}`（登録した順。固定の `cli`・`unknown` が先。`scopes` は 2章の表の順） |
| `set_agent_scopes` | `client`（識別子）、`allow?`・`deny?`（スコープ名の配列）、`expect?`（確認の画面で見せた変更前のスコープ。いまと違えば書かずに `conflict`（`changed`）） | `{updated, client, before, scopes}`。変えたら画面に `changed` を出す |
| `list_changes` | `client?`、`unseen?`（真偽）、`before?`（記録の ID）、`limit?` | `{changes: [...], next_before}`。形は 7.2 |
| `preview_revert` | `change_id` | `{id, operation, client, created_at, state, token, changed_since, effect}`。何も書かない。形は 7.3 |
| `revert_change` | `change_id`、`force?`（真偽）、`expect?`（`preview_revert` の `token`） | `{updated, id, operation, state, revert_id, effect}`。書いたとき（と、すでに戻っていた記録を取り消し済みにしたとき）は画面に `changed` を出す |

- エラーは 4.3 と同じ形：知らないスコープ名・形の誤りの ID・`limit` が 0 は `invalid_argument`、クライアントや記録が無いのは `not_found`。
- `list_changes`・`preview_revert` は何も変えないので、依頼を送った後に接続が切れても、読み取りと同じく 1 回だけ自動でやり直す（4.3 の `result_unknown`）。`set_agent_scopes`・`revert_change` は書き込みと同じく送り直さない。

- `append_note` の `heading` は、あれば現在時刻の見出しを入れてから追記する（CLI の `--heading [TITLE]`。`""` は件名なしの `## HH:MM`）。省略か `null` なら見出しを入れない。
- `edit_note` の `replace`・`delete`・`move_to` が 0 個か 2 個以上なら `-32602`。`line`・`move_to` は 0 以上の整数（0 の意味はコアが `invalid_argument` にする）。
- `updated` は 3.2 の意味（実際に何かを変えたか）。`updated: false` の成功では、画面への `changed` を出さない。
- `search` は上の表のとおり（4.2 に返り値）。`export_handoff` も同じ（4.2。コアの `domain::handoff::export`。2026-11-05 決定。YUN-70）。読み取りの操作なので、アプリ経由でも `changed` を出さず、依頼を送った後に接続が切れても 1 回だけ自動でやり直す（4.3 の `result_unknown`）。
- 権限・入力中・持ち越し・保存の失敗中の判定とエラーは、3章・3.2 のとおり（コアが行う。直接モードと同じ）。内部のエラーの文は応答に出さず（4.3）、読み書きの失敗は `storage_unavailable`（`message` は、何も変えていないことと、アプリを確かめるよう伝える）にする。
- データベースを使えるまで待つのは最大 5 秒。保存先の変更・復元の最中などで使えなければ、何も反映せずに `busy`（4.3）。アプリが止め始めていて始めなかった依頼も `busy`。`hello` の後で依頼が時間切れになった接続・止めるときに閉じる接続には、依頼を待たずに `busy`（`id` が `null` の失敗の応答）を 1 行返してから閉じる（後から依頼を送ったクライアントが `result_unknown` と取り違えず、そのまま送り直せるため。アーキテクチャ設計書 4.6）。

### 4.5 MCP サーバー（`yunagi mcp`。2026-11-04 決定。YUN-92）

作り方（薄い層にする、SDK を使わない理由、仕様の版）はアーキテクチャ設計書 4.7。ここは入出力の決まり。MCP のツールは、4.4 の操作を同じ名前・同じ引数で公開するだけで、返り値（4.2）とエラー（4.3）はローカル API・直接モードの JSON をそのまま載せる。

**起動と終わり**

- `yunagi mcp [--client ID]`。`ID` は 2章の識別子（`claude-code` など。形は 4.4 の `client` と同じ）。無ければ `unknown` クライアント（`read` のみ）。`--json` は無視する（引数の誤りも、`--json` を付けていても標準エラーにテキストで出し、標準出力には書かない）。
- 識別子の形の誤りは、起動のときに一度だけ、標準エラーに `invalid_argument` を書いて終了コード 2 で終わる（標準出力には何も出さない）。ツールを呼ぶたびにエラーにはしない。
- 標準入力が閉じられたら終了コード 0 で終わる。標準出力に書けなくなったとき（クライアントが閉じた）と、標準入力を読めなくなったとき（入出力の失敗。標準エラーに理由を書く）は 1。
- 既定ディレクトリを求める環境変数が無いときは終了せず、標準エラーに 1 行（4.3 の `storage_unavailable` の `message`）を書き、ツールを呼ぶたびに同じ `storage_unavailable`（探した環境変数の名前つき）を返す。起動に失敗してエージェントが何も見えないより、呼んだ結果に理由が出るほうが利用者に伝わるため。
- 標準出力には MCP のメッセージ以外を書かない。警告（アーキテクチャ設計書 4.5 の 3。プロセスごとに 1 回）と内部の失敗の診断は標準エラー（接続の道具を作るときに出たものは起動のときに、そのあとのものは各ツールの呼び出しの後に出す）。

**メッセージ**：JSON-RPC 2.0 を、1 行 1 メッセージ（途中に改行を含まない）で、標準入力から読み、標準出力へ書く。接続の状態は持たず、握手の前の `tools/call` も受ける。

- `id` は文字列か数で、受けたとおりに返す。`id` の無いもの（通知。`notifications/initialized` など）と、クライアントからの応答には、何も返さない。
- JSON でない行は `-32700`（`id: null`）、まとめての依頼（配列）・オブジェクトでない行・`method` の無い行・`id` が文字列でも数でもないものは `-32600`（`id` が読めるときは、`method` の無い行の応答にもその `id` を付ける）、知らない `method` は `-32601`、`params` がオブジェクトでない・`tools/call` の `name` が無い・知らないツール名は `-32602`。いずれもサーバーは終了せず、次の行を読む。
- 標準入力が改行の無いまま閉じられたときの最後の行も、1 行として処理する（最後の改行を付けずに閉じるクライアントの依頼を、黙って捨てないため）。
- 1 行の上限は 8 MiB（ローカル API の依頼の上限 4 MiB（アーキテクチャ設計書 4.6）の 2 倍）。超えた行は、残りを読み捨てて `-32600`（`id: null`）を返し、次の行から続ける（上限を超える行は最後まで読み込まずに捨てるので、その依頼の `id` は分からない）。
- 依頼は 1 件ずつ順に処理する（アーキテクチャ設計書 4.7）。そのため、`tools/call` が接続の手順で待っている間（最大 5 秒）に届いた `ping` と `notifications/cancelled`（キャンセル）は、その呼び出しが終わるまで読まれず、キャンセルは何もしない（反映済みの書き込みを途中で止められないため。応答は必ず返る）。

**握手と版**：仕様の 2 つの世代に答える（確かめた日と出典はアーキテクチャ設計書 4.7）。

| 依頼 | 答え |
| --- | --- |
| `initialize`（`2025-11-25` 以前） | `protocolVersion` が `2025-11-25`・`2025-06-18`・`2025-03-26`・`2024-11-05` のどれかならその版、それ以外なら `2025-11-25`。あわせて `capabilities`（`{"tools": {}}`）、`serverInfo`（`name: yunagi`、`title`、`version`＝`yunagi --version` の版）、`instructions`。`protocolVersion` が無ければ `-32602` |
| `server/discover`（`2026-07-28`） | `supportedVersions`（`["2026-07-28"]`）、`capabilities`、`_meta` の `io.modelcontextprotocol/serverInfo`、`instructions`、キャッシュの案内（`ttlMs` 300000・`cacheScope: public`） |
| `ping` | `{}` |
| `tools/list` | 下記のツール。最新の仕様の依頼にはキャッシュの案内（`ttlMs` 300000・`cacheScope: public`）を付ける |
| `tools/call` | 下記 |

- 依頼の `params._meta` に `io.modelcontextprotocol/protocolVersion` があるものは最新の仕様の依頼で、結果に `resultType: "complete"` を付ける。値が `2026-07-28` 以外なら `-32022`（`data.supported`・`data.requested`）。ない依頼（旧い仕様）の結果には、`resultType` もキャッシュの案内も付けない。`_meta` の版が旧い仕様の版（`2025-11-25` 以前の 4 つ）の依頼は、断らず、旧い仕様の依頼として答える。
- `instructions`（エージェントへの案内。英語）：Yunagi は利用者の日次の作業ノートであること、先に `get_board`・`open_board` で項目の ID を得ること、権限は利用者が Yunagi で決めるので `permission_denied` は利用者に伝えること、エラーの次の動き（4.3 の表：`busy` はそのまま送り直す、`user_editing` はしばらくして、`changed` は読み直して、`result_unknown` は読んで確かめてから、そのほかの `api_error` は送り直さず利用者に伝える）。

**ツール**：`tools/list` は次の 15 個（3章の表のうち A-07・A-08 を除いたもの。A-06 の `export_handoff` は YUN-70 で足した）。この順に返す。

`get_board`・`open_board`・`get_note`・`search`・`add_todo`・`edit_todo`・`complete_todo`・`reopen_todo`・`add_blocker`・`edit_blocker`・`resolve_blocker`・`append_reflection`・`append_note`・`edit_note`・`export_handoff`

- 各ツールに、`name`・`title`・`description`（英語。必要なスコープ、エラーの次の動き、やり直してよいかを書く）・`inputSchema`（引数は 4.4 の表と同じ名前で、`additionalProperties: false`。`edit_note` の「ちょうど 1 つ」は `description` に書く。`line`・`move_to` はスキーマでは 1 以上とし、0 はコアが `invalid_argument` にする）・`annotations.readOnlyHint`（読み取りの 5 つ（`get_board`・`open_board`・`get_note`・`search`・`export_handoff`）だけ `true`）を付ける。スキーマは手で書く。引数の読み取りと判定は `ops::Call::parse` とコアが行い、スキーマから作り直したり検査したりしない（決まりを 2 か所に書き写さないため）。名前が食い違わないことは試験で確かめる（スキーマの名前を `Call::parse` が断らないこと、スキーマに無い名前を断ること）。
- 後からローカル API の操作が増えても、この一覧に足すまでは MCP に出ない（呼ぶと `-32602`）。

**`tools/call` の結果**

- 成功：`{"content": [{"type": "text", "text": "<返り値の JSON を 1 行にしたもの>"}], "isError": false}`。返り値は 4.2・4.4 のとおり。`structuredContent` と `outputSchema` は付けない（同じ中身を 2 回返すと、エージェントの文脈に 2 倍入るため。`text` を JSON として読めば足りる）。
- 失敗：`Connector::call` が返すエラー（4.3 のすべてのコード。`busy`・`api_error`（`reason` つき）・`default_dir_mismatch` を含む）は、`isError: true` で、`text` に `{"error": {...}}`（CLI の `--json` の失敗と同じ形）を入れる。アプリが返した `error.data.error`（4.4）のオブジェクトはそのまま載せる。サーバーは終了せず、次の依頼を受け付ける。
- プロトコルのエラー（JSON-RPC の `error`）にするのは、知らないツール名と `name` の欠けだけ（仕様の Error Handling：エージェントが直せない依頼の形の誤り）。引数の誤り（知らない名前・型違い・形の誤りの ID や日付）は、エージェントが直して送り直せるので、ツールのエラー（`invalid_argument`）にする。
- `arguments` が無いか `null` なら `{}` として扱う。オブジェクトでなければ `invalid_argument`。
- 引数を JSON にした大きさがローカル API の依頼の 1 行の上限（4 MiB）から包みの余裕（16 KiB）を引いた大きさを超えるときは、接続する前に `invalid_argument`（`The arguments are too large. Send less text at a time.`）。直接モードでは通る大きさの依頼が、アプリ経由では断られる（経路で結果が変わる）ことを避けるため、CLI の標準入力の本文（4.1）と同じ考え方で、MCP から先に断る。
- 書き込みの二重を避ける決まり（アーキテクチャ設計書 4.4・4.6）：引数が揃ってから `Connector::call` を呼ぶ。送った後に切れた書き込みは送り直さず `result_unknown`（`isError: true`）を返す。

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

区間と障害の求め方の細部（2026-11-03 決定。コアの `domain::note_send::request_context`）：区間は送り元のノートの日（持ち越した TODO では元の日）の、いまの本文で求める。「同じ見出しの区間の行から送った障害」は、区間の行に送信済みの印を付けている障害（データモデル設計書 3章 note_send）で、行の順に並べ、同じ障害は1件にする。持ち越した障害は、最後の持ち越し先の項目の状態で未解消かを決め、その項目を載せる（持ち越し元の状態は持ち越した後に変わらないため）。持ち越した障害の元の日の項目を削除しても、持ち越し先が未解消なら載せる（2026-11-05 決定。YUN-70。削除された項目の記録の行は、生きている記録の後に照合する。データモデル設計書 3章 note_send）。持ち越し先を持たない削除済みの障害は載せない。同じ項目を指す記録が複数あっても、障害は項目ごとに1件にする（持ち越し先を重ねて調べない）。コアは区間の行を空白だけの行も含めてそのまま返し、行の整え方は次のとおり。

形式の細部（2026-11-05 決定。YUN-70。コアの `domain::handoff`）：

- 対象は TODO だけ。障害の ID は `invalid_argument`（4.2）。完了・取りやめ・持ち越し済みの TODO も書き出せる。
- 見出し（`## Task`・`## Context`・`## Related blockers`）と末尾の注記は、画面の言語の設定にかかわらず英語に固定する（貼り付けた先のエージェントへ同じ形で渡すため。アーキテクチャ設計書 6.4 の書き出しと同じ）。中身の無い節は出さない。節の間と注記の前は空行 1 行。末尾は改行 1 つ。
- Task：ID で指した TODO のいまの本文（持ち越した TODO を指せば、その持ち越し先の本文。持ち越し済みの元の TODO を指せば、元の本文）。
- Context：区間の行を、次のとおり整えて 1 行ずつ `- ` で並べる。区間に時刻付き見出しがあれば、各行の前に `<見出しの文字>: `（例：`10:00 定例: `）を付ける（見出しの無い前置きの区間では付けない）。
  - 除く行：空白だけの行、見出しの行（時刻付きでない `## 良かったこと` など）、区切り線、コードの囲みの印の行、項目へ送った行（送信済みの印の付いた行。送り元の行を含む。送り先の項目の種類・削除の有無を問わない）。送った行は Task や Related blockers にすでに出る（またはほかの TODO として扱われる）ので、文脈に重ねない。
  - 整える：行頭の空白、リストの印（`-`・`*`・`+`・`1.`・`1)`）、チェック欄（`[ ]`・`[x]`）、前後の空白を除く（項目の本文を作るときと同じ。データモデル設計書 3章 note_send）。字下げの深さは残さない。
  - コードの囲みの中の行（囲みの印の行は除く）は、前後の空白を除いてそのまま載せる（ログなどを、リストの印のように見える行も含めて崩さないため）。囲みは区間の中だけで数える（区間をまたぐ囲みは考えない）。
  - 見出しの文字・行の長さ・行数に上限は設けない（ノートの区間は人が書く量のため）。
- 見出しの日付：送り元のノートの日が書き出す日（呼び出しの時点の論理日付の今日）と同じなら `## Context (from today's notes)`、違えば `## Context (from notes on YYYY-MM-DD)`（持ち越した TODO や過去の日の TODO で、「today's」と書くと誤るため）。
- Related blockers：未解消の障害のいまの本文を `- ` で並べる（行の順）。
- 末尾の注記の日付：書き出す日の論理日付の今日（`today`・設定の切り替わり時刻で求める。`get_board` の日付の既定と同じ）。
- ノートから送られていない TODO（送り元の行が見つからない場合を含む）は、Task と末尾の注記だけ。
- 1つの項目に note_send の記録が複数ある壊れたデータ（通常は項目1件につき記録1件）：送り元は id の最も小さい記録とする（`source_line` と同じ。最初に作った記録）。印は記録ごとに付くので、依頼文の Context から除く行には、すべての記録の行を数える。

出力先：CLI は標準出力（4.1）、MCP は `export_handoff` の返り値（4.2）、アプリは Mod+Shift+A でクリップボード（操作仕様書 2章。Rust 側で書く。アーキテクチャ設計書 4.3）。クリップボードの書き込みは、3 OS で実機の確認が要る（未確認。テスト計画書 4.2 の実機の項目）。

## 6. 導入（A-07）

`yunagi agent setup` は、検出したエージェントの設定に MCP サーバーを登録する。

| 対象 | 登録方法 | 設定ファイルの形式 |
| --- | --- | --- |
| Claude Code | `claude mcp add --scope user --transport stdio yunagi -- yunagi mcp --client claude-code`（`--` の後ろがそのままサーバーのコマンドと引数になる）。`agent setup` は、設定ファイルを直接書き換えず、この `claude` コマンドを実行して登録する | `~/.claude.json`（`user` スコープ）の `mcpServers.yunagi`：`{"type": "stdio", "command": "yunagi", "args": ["mcp", "--client", "claude-code"]}`（`env` は付けない） |
| Codex | `~/.codex/config.toml` に `[mcp_servers.yunagi]` の表を追記する（`codex mcp add` は使わない：環境変数の指定は `--env KEY=VALUE`（値を渡す）だけで、名前を転送する `env_vars` を指定する引数が無く、すでに同じ名前があると確認なしに置き換えて `env_vars` も消すため。2026-11-04、codex-cli 0.166.0 の `codex mcp add --help` と実行で確かめた） | `[mcp_servers.yunagi]` の `command = "yunagi"`、`args = ["mcp", "--client", "codex"]`、`env_vars = [...]`（下の「登録するコマンドに環境変数を含めるか」） |

確かめた日と出典（2026-11-04。公式ドキュメントだけ）：

- Claude Code：<https://code.claude.com/docs/en/mcp>（`claude mcp add` の書式、`--scope`、保存先、`.mcp.json` の形）、<https://code.claude.com/docs/en/env-vars>（環境変数の引き継ぎ）。
  - `--scope` の既定は `local`（いまのプロジェクトだけ）。Yunagi は特定のプロジェクトに結び付かないので `user`（すべてのプロジェクトで使える）にする。
  - `~/.claude.json` はエージェントのほかの状態も入るので、`agent setup` は直接編集せず、`claude mcp add` を通す。`claude` コマンドが見つからないときは、実行すべきコマンドを表示して案内する。
  - 公式ドキュメントに書いてあるのは、「同じスコープで同じ名前の `claude mcp add` は `MCP server <名前> already exists in local config` で失敗する」ことだけ。以下は、実機（Claude Code 2.1.318、Linux、一時フォルダを `HOME` にした環境。2026-11-04）で確かめた挙動で、公式ドキュメントには無い。
    - 同じ名前の `claude mcp add --scope user` は、**上書きせず**、`MCP server yunagi already exists in user config` を出して終了コード 1 になる。`claude mcp remove --scope user yunagi` は外して終了コード 0、無ければ `No MCP server named "yunagi" in user scope` で終了コード 1。
    - `claude mcp get`・`claude mcp list` は、**サーバーを起動して接続を試す**（health check。1 回の `get` で 2 回起動した）うえ、全スコープを見て、無い名前は終了コード 1。登録の有無を調べるだけの用途には使わない（`yunagi mcp` が起動されてしまう）。
    - 設定ファイルの場所は、`CLAUDE_CONFIG_DIR` が無ければ `$HOME/.claude.json`、あれば `$CLAUDE_CONFIG_DIR/.claude.json`（`CLAUDE_CONFIG_DIR` が先）。`claude mcp add` は最上位の `mcpServers.yunagi` に `{"type": "stdio", "command": "yunagi", "args": [...], "env": {}}` を書く（`env` は空の表）。Claude Code は書き換えるたびに、自分でも `<設定の置き場所>/backups/` にコピーを作る。
  - 扱い方（`agent setup` が決めて実装した）：登録済みかは、設定ファイルを**読むだけ**で調べる（最上位の `mcpServers.yunagi`。書き換えないので「直接編集しない」には反しない）。(1) 同じ内容（`type` は無いか `stdio`、`command` が `yunagi`、`args` が上の 3 つ、`env` は無いか空）なら、何も変えず、確認もバックアップも出さずに「登録済み」と伝える。(2) 違う内容なら、変更内容の表示に今の登録の `command` と `args` を出し（`env` は名前だけで値は出さない）、確認のうえで、バックアップ → `claude mcp remove --scope user yunagi` → `claude mcp add ...` の順に実行する（`--scope user` の `yunagi` だけを対象にし、ほかの登録は触らない）。付け直しに失敗したら、外した元の登録を `claude mcp add-json --scope user yunagi '<元の JSON>'` で戻してから失敗を伝える。ただし元の登録が、コマンドと引数だけの単純なもの（`type` は無いか `stdio`、`env` は無いか空、`type`・`command`・`args`・`env` 以外の欄が無い）でないとき（`env` に値がある、`headers` にトークンがある http のサーバー、など）は、秘密をコマンドの引数（プロセスの一覧から見える）に載せないよう、自動では戻さず、元の登録の要約（値は伏せる）と参考のコピーの場所を伝えて、利用者に戻してもらう。戻すのに失敗したときも同じ要約を伝える。参考のコピーを作れない（設定ファイルがあるのに読めない）ときは、確認で約束したコピーを作れないので、何も変えずに止める。(3) 設定ファイルが無い、または `mcpServers` が無ければ、登録なしとして `add` する。(4) 設定ファイルを読めない・JSON として壊れている・形が違うときは、調べられないので、先に `add` を試し、失敗したら `claude` が出した文をそのまま利用者に伝える（黙って上書きしない・`remove` もしない）。`claude` コマンドの実行には標準入力を渡さず、終了コードが 0 以外なら失敗として、標準エラーと標準出力の文を伝える。
- Codex：<https://developers.openai.com/codex/mcp>（公式ドキュメントの移転先は <https://learn.chatgpt.com/docs/extend/mcp?surface=cli>。`[mcp_servers.<名前>]` の表、`command`・`args`・`env`・`env_vars`・`cwd`、プロジェクトごとの `.codex/config.toml` は信頼したプロジェクトだけ）、<https://learn.chatgpt.com/docs/config-file/config-reference>（`mcp_servers.<id>.*` の各項目）。
  - 書き換えは `toml_edit`（コメントと並びを保つ TOML の編集。版の選び方は `Cargo.toml`）で行う。`[mcp_servers.yunagi]` の `command`・`args`・`env_vars` の 3 欄だけを足す・直し、その表のほかの欄（`enabled`・`startup_timeout_sec`・`env` など）、ほかの表、コメント、行末のコメント、改行の形（CRLF のファイルは CRLF のまま）は変えない。`mcp_servers` が無ければ `[mcp_servers]` の見出しは出さず `[mcp_servers.yunagi]` だけを末尾に足す。インライン表（`mcp_servers = { ... }`）・ドット付きのキー（`yunagi.command = ...`）の書き方でも壊さない。`[mcp_servers.yunagi]` が既にあって内容が違えば、変更内容の表示に今の 3 欄の値を出し、確認のうえで置き換える。同じ内容なら何もしない。TOML として読めないファイル・`mcp_servers` や `yunagi` が表でないファイルには**書かずに止め**（行番号と理由だけを伝える。ファイルの内容は出さない。トークンなどが入りうるため）、もう一方のエージェントの登録は続ける。確認を待つ間にファイルが変わっていたら、書かずに止める。
  - 3 欄が同じなら「登録済み」で何もしない。その表に `enabled = false` があれば（利用者が無効にしている）、変えず、Codex が起動しない旨を伝える。3 欄が違って書き換えるときも `enabled = false` は残し、変更内容の表示に同じ旨を添える。
  - `[mcp_servers.yunagi]` が HTTP のサーバーとして書かれている（`url`・`bearer_token_env_var`・`http_headers`・`env_http_headers` のどれかがある）ときは、`command` を足すと Codex が設定ファイル全体を読めなくなりうるので、書かずに止め、利用者に自分で外すか名前を変えるよう伝える。改行がすべて CRLF のファイルは、CRLF のまま書く。LF と CRLF が混ざったファイルは、`toml_edit` が読み込むときに CRLF を LF にそろえてしまい、ほかの行の改行を変えずには書けないので、書かずに止め、改行をそろえるよう伝える。変更内容の表示は、書き込む文書から作る（表示と書き込みがずれない）。
  - 書き込みは、同じフォルダの一時ファイルに書いてから名前を変える（前の実行が残した一時ファイルと名前がぶつかったら、消さずに別の名前にする）。元のファイルの権限と、UNIX 系では持ち主とグループ（合わせられなければそのまま続ける）を引き継ぐ。ACL・拡張属性・ハードリンクは引き継がない（名前を変えて置き換えるため）。シンボリックリンク（dotfiles の管理でよくある形）ならリンクを置き換えず指す先の実体に書く。ファイルやフォルダが無ければ作る（バックアップは取らない）。
  - 設定の置き場所は、`CODEX_HOME` があれば `$CODEX_HOME/config.toml`、無ければ `$HOME/.codex/config.toml`（2026-11-04、codex-cli 0.166.0 で、どちらも一時フォルダで確かめた。`codex mcp add` の書き込み先もこの場所）。`agent setup` が書くのは利用者全体の設定だけで、プロジェクトごとの `.codex/config.toml` は書かない。
  - `codex mcp get`・`codex mcp list` は、設定を読んで表示するだけで、サーバーを起動しない。`env_vars` の名前は値を伏せて表示される（`HOME=*****`）。
- Windows 固有の手順（`cmd /c` での包み方など）は、どちらの公式ドキュメントにも MCP の登録としては見つからなかった。開発は Linux でしか行えず、Windows での登録（`claude.exe`・`codex` の実行、`%USERPROFILE%\.claude.json`・`%USERPROFILE%\.codex\config.toml` の場所、`yunagi.exe` を包まずにそのまま登録して起動できるか）は**未確認**（実装は `PATH` の検索で `.exe`・`.cmd`・`.bat` の順に試し（拡張子の無いファイルは試さない。npm が `claude.cmd` の隣に置く sh 用のファイルを起動してしまわないため）、ホームは `USERPROFILE` から求めるが、実機では動かしていない）。YUN-87 の完了の条件に回した。

**書き込む前の公式ドキュメントの再確認（2026-11-04、YUN-93）**：上の出典（<https://code.claude.com/docs/en/mcp>、<https://code.claude.com/docs/en/env-vars>、<https://developers.openai.com/codex/mcp> の移転先、<https://learn.chatgpt.com/docs/config-file/config-reference>）をもう一度読み、`claude mcp add` の書式・`--scope` と既定・保存先 `~/.claude.json`・`CLAUDE_CODE_MCP_ALLOWLIST_ENV` の説明・Codex の `[mcp_servers.<名前>]` の `command`・`args`・`env`・`env_vars`・`cwd`・`codex mcp add` の引数（名前の転送の引数は無い）が、この章の記述と食い違わないことを確かめた。確かめたエージェントの版は Claude Code 2.1.318、codex-cli 0.166.0。各エージェントの設定形式は変わりうるので、`agent setup` を直すときは、この確認をもう一度行う。

**`agent setup` の動き（2026-11-04 決定。YUN-93）**：コードは `crates/cli/src/agent_setup/`。

- 対象：`--claude-code`・`--codex` の指定があれば、そのエージェントだけ（検出しなくても対象にする）。どちらも無ければ検出したもの。検出は、Claude Code は `claude` コマンドが `PATH` にあるか設定ファイルがあること、Codex は `codex` コマンドが `PATH` にあるか設定のフォルダがあること。どれも見つからなければ `not_found`（終了コード 4）で、何も書かない。`claude` コマンドが `PATH` に無いときは、実行すべきコマンドを表示して案内する（何も登録していない。`--claude-code` で名前を指定されたときは終了コード 4（`not_found` と同じ）にして、スクリプトが登録済みと取り違えないようにする。検出で選ばれただけのときは 0）。`--json` は受け付けず `invalid_argument`（確認の質問を含むため）。
- 流れ：検出 → 変更内容を作る（何も書かない）→ 警告と変更内容を表示 → 確認 → 書き込み。全部が「登録済み」なら確認を出さずに終わる。変更内容の表示には、書くコマンドまたは表、バックアップの置き場所、`yunagi` が `PATH` に無いときの注意（エージェントが起動できない）、環境変数の値を保存しないことを出す。環境変数の値は、表示にも登録にも出さない。
- 確認は**対話端末（TTY）でだけ**得る（標準入力と標準出力の両方が端末のとき。質問は標準出力に出るので、`> log.txt` や `| tee` で質問が見えないまま答えを待たないため。2章の確認と同じ考え方：誤って確定しないための確認で、防御の境界ではない。標準入力を擬似端末につなぐなどすれば、エージェントのシェルからも通せる。登録で得られるのは、`unknown` でなく `claude-code`・`codex` という識別子の、既定の `read` だけで、書き込みの権限は与えない。2章）。端末でない実行では、変更内容を表示したうえで、何も書かずに `permission_denied`（`required_scope` なし。終了コード 3）で終わる。確認の質問は `y`・`yes`（大文字小文字は問わない）だけを承諾とし、それ以外は「取りやめた」として何も書かずに終了コード 0。`--yes` のような確認を省く指定は設けない。
- 終了コード：全部成功・取りやめ・登録済みは 0、一部または全部の書き込みや準備の失敗は 1（失敗したエージェントの理由を標準エラーに出す。ほかのエージェントの登録は続ける）。
- Linux で `XDG_DATA_HOME` が設定されているときは、Claude Code の変更内容に「`CLAUDE_CODE_MCP_ALLOWLIST_ENV=1` で起動すると `XDG_DATA_HOME` が渡らない」注意を 1 行添える（下の実機の結果）。

登録前に変更内容を表示し、利用者の確認を得てから書き込む。既存の設定はバックアップする。置き場所は元のファイルの隣の `<ファイル名>.yunagi-backup-<日時>`（日時は `20261104-120000` の形。上書きしない：同じ名前があれば `-2`・`-3` を付けて新しく作る。実行のたびに新しく作る。設定ファイルにはトークンなどが入りうるので、UNIX 系では本人だけが読み書きできる権限（0600）で作ってから中身を書き、書き終えて元のファイルの権限に合わせる。合わせられなくても 0600 のまま残るので、元より広くはならない。新しく作る `config.toml` も 0600）。Codex は `config.toml` の中身を、書き込む直前に読み直した内容でコピーしてから、`[mcp_servers.yunagi]` だけを足す・直す。バックアップを作れなければ、何も書かない。Claude Code は `claude` コマンドに任せるので、`~/.claude.json` のコピーは参考として残すだけにする（Claude Code が動いている間は同じファイルを書き換えるため、丸ごと戻すと直前の状態を失う）。取り消すときは、コピーを戻さず `claude mcp remove`（`--scope user` の `yunagi`）を案内する。Codex の取り消しは、コピーを戻すか、`[mcp_servers.yunagi]` の表を消す。

**登録するコマンドに環境変数を含めるか（2026-11-04 決定。同日、YUN-7 で公式ドキュメントを確かめて確定）**：環境変数の**値**は含めない（Codex の名前だけの転送は下の例外）。`XDG_DATA_HOME` などの値を登録に書くと、既定ディレクトリの絶対パスを設定ファイルに書き残すことになり、既定ディレクトリごと移したとき（新しい端末への引っ越しなど）に古い場所を指すため（アーキテクチャ設計書 6章と同じ理由）。また、`agent setup` を実行した端末の値が、アプリを起動する環境の値と同じとは限らない。代わりに、`agent setup` は書き込む前に、アーキテクチャ設計書 4.5 の別の候補を調べ、ずれていれば（別の候補を使っているプロセスがいる、または自分の既定ディレクトリが無い）変更内容の表示に警告を添える：この端末で求めた既定ディレクトリと、別の場所でアプリが動いていること、アプリを起動する環境とエージェントを起動する環境で同じ `XDG_DATA_HOME` などを使うこと。警告が出ても、利用者が確認すれば登録できる。

**エージェントが環境変数を引き継ぐか（2026-11-04、YUN-7 が公式ドキュメントで確かめ、YUN-93 が実機で確かめた結果）**：エージェントが MCP サーバーに、自分の環境変数のうち既定ディレクトリの変数を引き継がないと、その変数がないと `yunagi mcp` が標準の場所を見て、アプリと違う場所になりうる（検出できない向き。アーキテクチャ設計書 4.5）。

実機の確かめ方：環境変数の**名前**と、指定した名前の値の有無だけを記録する偽のサーバー（シェルスクリプト）を登録し、エージェントに起動させて、起動された側の環境を見た。環境は一時フォルダの `HOME`・`XDG_DATA_HOME`（本物の設定には触れない）。Linux だけの結果で、macOS・Windows は未確認。

- **Claude Code 2.1.318（Linux）：既定では引き継ぐ**。`claude mcp get`（接続の確かめでサーバーを起動する）で起動されたサーバーに、`XDG_DATA_HOME`・`HOME` と、確かめ用に足した任意の変数が渡った。公式ドキュメントに明記は無く（`CLAUDE_CODE_MCP_ALLOWLIST_ENV` の説明が、`1` にすると「シェルの環境を引き継ぐ代わりに」安全な基準の環境と設定の `env` だけで起動する、と書いているのみ）、既定の引き継ぎは実機で確かめた。
  - `CLAUDE_CODE_MCP_ALLOWLIST_ENV=1` のときの基準の環境（Linux）は、`HOME`・`LOGNAME`・`PATH`・`PWD`・`SHELL`・`SHLVL`・`TERM`・`USER` と `CLAUDE*` の変数。**`HOME` は渡り、`XDG_DATA_HOME` は渡らない**（確かめ用の任意の変数も渡らない）。`LOCALAPPDATA`（Windows）は未確認。
  - `CLAUDE_CODE_SUBPROCESS_ENV_SCRUB=1` のとき、公式ドキュメントは「認証情報を、変数の名前か値で見分けて、stdio の MCP サーバーの環境から外す」と書くが、実機では**`XDG_DATA_HOME` も渡らなかった**（`HOME` は渡り、任意の変数も渡った）。値を乱数のような文字列を含まない短いパスに変えても外れ、同じ値を持つ別の名前の変数は渡ったので、値ではなく名前で外れる（`XDG_*` をまとめて外すのかは未確認）。
  - したがって、利用者が `XDG_DATA_HOME` を設定していて（アプリもその値で起動していて）、Claude Code をこの 2 つのどちらかで起動すると、`yunagi mcp` は標準の場所を見る。これは 4.5 の**検出できない向き**（アプリのほうが標準外の場所）にあたり、自分の既定ディレクトリが無い・データが無いときは `storage_unavailable` で気づけるが、標準の場所に以前の古いデータベースが残っていると気づけずにそれを開く。Claude Code の `env` には値しか書けず（名前の転送は無い。`${XDG_DATA_HOME}` の展開は未設定のとき文字列のまま渡る）、値を書くと絶対パスが残るので、登録では防げない。`agent setup` は、Linux で `XDG_DATA_HOME` が設定されているとき、変更内容の表示にこの 2 つの設定を名指しした注意を 1 行添える。アプリを起動する環境とエージェントを起動する環境で同じ `XDG_DATA_HOME` を使う決まり（アーキテクチャ設計書 4.5）に、「Claude Code を上の 2 つで起動するなら、`XDG_DATA_HOME` を使わない」が加わる。
- **Codex（codex-cli 0.166.0、Linux）：既定では引き継がない**。公式ドキュメントに親の環境の扱いの記述は無い（<https://learn.chatgpt.com/docs/config-file/config-reference> の `env_vars` は「許可して転送する環境変数」、`env` は「サーバーに設定する値」）。実機では、`codex exec` が起動したサーバーに渡った環境は、基準の `HOME`・`LANG`・`LOGNAME`・`PATH`・`PWD`・`SHELL`・`SHLVL`・`TERM`・`USER`（Linux）と、`env_vars` に書いた名前だけだった。`XDG_DATA_HOME` は `env_vars` に書いたときだけ渡り、書かない任意の変数は渡らなかった。**`env_vars` に、設定されていない名前を書いてもエラーにならず**、サーバーは起動した（その名前はただ渡らない）。macOS・Windows の基準の環境は未確認。

**Codex の登録だけ、環境変数の名前を転送する指定を書く**（Codex は上のとおり `XDG_DATA_HOME` を引き継がないことが実機で分かったので、必要）。書く名前は、アーキテクチャ設計書 6章の「求める環境変数」（macOS は `HOME`、Windows は `LOCALAPPDATA`、Linux は `XDG_DATA_HOME` と `HOME`。コアの `default_dir_vars`）を、設定の有無に関わらず**すべて**書く（Linux は `env_vars = ["XDG_DATA_HOME", "HOME"]`、macOS は `["HOME"]`、Windows は `["LOCALAPPDATA"]`）。設定されているものだけにすると、`agent setup` の後で `XDG_DATA_HOME` を設定したときに転送されず、`yunagi mcp` が静かに標準の場所を見て、検出できない向き（4.5）のずれになる。設定されていない名前を書いても、Codex（0.166.0、Linux）はエラーにせず起動する（上の実機の結果）ので、すべての名前を書く形のままにする（「設定されている名前だけ書く」形には直さない）。macOS・Windows で同じかは未確認で、YUN-87 に回した（エラーになり Codex が起動できなくなると分かった場合は、設定されている名前だけを書き、後から `XDG_DATA_HOME` を設定したら `agent setup` の再実行が要る、と確認の表示に添える形に直す）。値は書かず、Codex が起動したときの自分の環境から渡すので、絶対パスは設定に残らず、引っ越しで古くもならない（上の「値を含めない」理由に当たらない）。

Claude Code にも設定の `env` で `${XDG_DATA_HOME}` のように環境変数を展開する書き方があるが、未設定の変数は展開されず `${...}` の文字列のまま渡される（<https://code.claude.com/docs/en/mcp>）ので、使わない。Claude Code は既定で引き継ぐので何も足さず、上の 2 つの設定で外れる場合は 4.5 の検出と注意に頼る。

値そのものを登録に含める（`env` に書く）エージェントは、今は無い。将来、引き継がないと確かめたエージェントを足すときは、名前の転送ができるならそれを使い、できないときに限り、実行時に設定されている `XDG_DATA_HOME`（Linux）・`LOCALAPPDATA`（Windows）の値だけを含める（`HOME` は含めない。引き継がれないと `yunagi mcp` がそもそも動けず、ほかの登録も壊すため）。含める内容は確認の表示に出し、登録済みの値が現在の環境の値と違うときは変更内容の表示で知らせる（書き残す先はエージェントの設定で、`agent setup` を再実行すれば更新できる）。

## 7. 変更履歴（A-08）

- todo・blocker・append・note_edit に該当する操作は、MCP・CLI のどちらからでも、すべて change_log に記録する（operation は 3章の命名規則に従う）。ただし、何も変えずに成功を返した操作（解消済みの障害への `resolve_blocker`、今と同じ本文の `edit_todo`・`edit_blocker` など。3章）は記録しない。
- 利用者が画面で項目の状態を変えてから保存されるまでの間に届いた、同じ状態にする `complete_todo`・`reopen_todo`・`resolve_blocker` は、受け付けて成功を返すが、利用者の保存が書き込まれた時点で利用者の変更として扱い、その操作の記録から状態と完了時刻の欄を外す（何も残らなければ記録を消す。アーキテクチャ設計書 4.1）。利用者の保存が先に届いた場合に記録されないのと同じ結果にするため。
- MCP・CLI から作成された項目には `origin = agent` と agent_client_id（CLI は `cli`）を記録し、画面で印を表示する。
- `yunagi agent revert` は取り消しを実行し、対象がその後に人によって変更されている場合は `conflict`（`reason: changed`。終了コード 5）を返す（`--force` で上書き）。項目では、その記録の操作が変えた欄だけで判定し、取り消しもその欄だけを戻す（ほかの欄に人が加えた変更は残る。データモデル設計書 change_log）。取り消しの確定には 2章の確認が必要。動きは 7.3。

### 7.1 権限の管理（`agent permissions`。2026-11-04 決定。YUN-94）

コードは、コアが `domain::agent_admin`、CLI が `crates/cli/src/agent_admin.rs`、API の操作が 4.4。

- 見る：`yunagi agent permissions [CLIENT]`。登録されているクライアントと許可したスコープ・最後の接続の一覧（出力は 4.1）。確認は要らない。`CLIENT` が登録されていなければ `not_found`（終了コード 4）。
- 変える：`--allow S`・`--deny S`（繰り返せる。`todo,blocker` のようにコンマで区切ってもよい。`S` は 2章のスコープ名）。`CLIENT` が要る。結果の並びは 2章の表の順で、足すスコープがすでにあり、外すスコープがすでに無ければ、何も書かない（`updated: false`）。`read` も外せる（外すとそのクライアントは何も読めない。管理の操作は使える。2章）。権限の変更は人の操作なので、change_log には記録しない。
- 引数の誤り（`invalid_argument`）：知らないスコープ名、同じスコープを足すことと外すことの両方に指定、`unknown` に `read` 以外を足す（2章）、`CLIENT` が無いのに `--allow`・`--deny` を指定（CLI。`--allow`・`--deny` が無ければ一覧になる）、API の `set_agent_scopes` で足すものも外すものも無い。`CLIENT` が登録されていなければ `not_found`。CLI は、確認の質問をする前に、分かる誤りを断る。
- 確認：対話端末（標準入力と標準出力の両方が端末）でだけ得る。変更内容（`now:` と `after:` のスコープ。`after:` はコアの `apply_scopes` で、書き込みと同じ計算。足すスコープで何ができるか）を表示し、変更前のスコープを `expect` に付けて書く（確認を待つ間に別のところで変わっていたら、書かずに `conflict`（`changed`））。`Apply this change? [y/N]` と聞く。`y`・`yes`（大文字小文字は問わない）だけを承諾とし、それ以外は「取りやめた」として何も書かずに終了コード 0。入力が閉じている・読めないときは、失敗（終了コード 1）。すでに目的のスコープなら、聞かずに `Nothing to change.` で終わる（0）。端末でなければ何も書かず、`permission_denied`（`required_scope` なし。終了コード 3。4.3）。確認は誤って確定しないための確認で、防御の境界ではない（2章）。
- 判定の順（CLI）：引数の誤り → `unknown` へ足す誤り → 端末か → `CLIENT` が登録されているか → 確認 → 書き込み。引数の誤りが権限（端末）より先なのは、直してやり直すしかないため（3.2 の例外と同じ）。
- 画面への知らせ：変えたら `changed`（`by: agent`）を出す。設定画面のエージェントの一覧が読み直される（初めての識別子の登録と同じ経路）。

### 7.2 変更履歴の一覧（`agent history`。2026-11-04 決定。YUN-94）

コアは `domain::history::list_changes`、API の操作は `list_changes`（4.4）。

- 新しい順（記録の ID の降順）に、エージェントの記録と、人の取り消しの記録（`operation` が `agent.revert`）の両方を返す。ID（ULID。データモデル設計書）は同じプロセスの中では作った順に増えるが、別のプロセス（アプリと CLI の直接モード）が同じミリ秒に作った記録の順は決まらない（`list_by_target` と同じ「おおよそ記録した順」。時刻は `created_at`）。`--client NAME` はそのクライアントの記録だけ（人の取り消しの記録はクライアントが無いので入らない。登録のない `NAME` は `not_found`）。`--unseen` は、エージェントによる未確認の変更（`actor = agent` で `seen_at` が空。データモデル設計書 change_log）だけ。`--limit N` は 1 以上（既定 50、200 を超える指定は 200。0 は `invalid_argument`）。`--before ID` はその記録より古いものだけ（続きを読む目印。返り値の `next_before` は、`limit` 件ちょうど返したときの最後の記録の ID で、それ未満なら `null`）。
- 1 件の形：`{id, created_at, actor, client, operation, target, summary, seen_at, reverted_at, revert_of}`。`target` は、項目なら `{type: "item", id, date, kind, text, deleted}`（`text` はいまの本文。項目が見つからなければ `id` だけ）、ノート・振り返りなら `{type: "note" | "reflection", date}`。`summary` は、項目の追加が `{type: "added", kind, text}`、項目の欄の変更が `{type: "fields", fields: [{name, before, after}]}`（変えた欄だけ）、ノート・振り返りが `{type: "text", before_chars, after_chars}`（本文は大きいので文字数だけ。差分は取り消しの計画で見る）、読めない記録が `{type: "unknown"}`。
- 一覧は、ノート・振り返りの記録の本文を読まず、文字数だけを SQLite に数えさせる（本文が大きい記録を、一覧のたびに読み込まないため）。一覧は確認済みの印（`seen_at`）を付けない。`seen_at` は変更履歴の画面で確認した時刻（データモデル設計書 change_log）なので、CLI で一覧を出しても未確認のまま残る。印を付ける口は、コアの `history::mark_seen`（7.4。2026-11-05 決定。YUN-81）。

### 7.3 取り消し（`agent revert`。2026-11-04 決定。YUN-94）

コードは、コアが `domain::history`（`preview`・`revert`）、CLI が `crates/cli/src/agent_admin.rs`、API の操作が `preview_revert`・`revert_change`（4.4）。

**何を取り消せるか**：エージェントの記録（`actor = agent`）。取り消しの記録（`agent.revert`）は取り消せない（`invalid_argument`。元の記録を取り消す）。取り消しは `before` を書き戻す新しい操作で、取り消し自体も記録する：`actor = human`、`agent_client_id` は空、`operation = agent.revert`、`revert_of_id` は元の記録、`before` は取り消し前のいまの値、`after` は戻した値（項目の欄は `fields_json` と同じ形のオブジェクト、本文は JSON の文字列。行き違いの判定が状態をこの形で読む。アーキテクチャ設計書 4.1）。元の記録には `reverted_at` を入れる。同じトランザクションで行う。

**記録の種類ごとの戻し方**

| 記録 | 戻すこと | 「人があとから変えたか」を調べる欄 |
| --- | --- | --- |
| 項目の欄の変更（`todo.edit`・`todo.complete`・`todo.reopen`・`blocker.edit`・`blocker.resolve`） | その記録の `after` に入っている欄だけを `before` に戻す（ほかの欄に人が加えた変更は残る） | `after` に入っている欄の、いまの値と `after` の値 |
| 項目の追加（`todo.add`・`blocker.add`） | 項目を削除する（画面の削除と同じ論理削除。`deleted_at` を入れる） | 本文・状態・取りやめた理由・解消メモ・完了時刻。並び順は調べない（並べ替えだけで必ず食い違うのを避けるため） |
| ノート・振り返りの本文の変更（`note.append`・`note.edit`・`reflection.append`） | 本文全体を `before` に戻す（版を進める） | 本文全体と `after` |

- 後の記録がある欄：エージェントが同じ欄を続けて変えた場合も、人の変更と区別せず、`changed` になる。新しい記録から順に取り消す。
- すでに戻っている：いまの値がすでに戻したい値と同じ（人が先に戻した、追加した項目をすでに削除した、本文がすでに前の本文）なら、何も書かずに、元の記録を取り消し済みにして成功を返す（`updated: false`、`state: already_restored`）。取り消しの記録は作らない。
- すでに取り消し済み（`reverted_at` がある、または取り消しの記録から指されている）：何も書かずに成功を返す（`updated: false`、`state: already_reverted`）。取り消しの応答が届かずに、人がやり直した場合を失敗にしないため。
- `--force` が越えるのは「人があとから変えた」（`changed`）だけ。確認で見せた値だけを上書きする：CLI は取り消す内容を見せたときの状態を表す `token`（`preview_revert` の返り値）を `expect` に付けて送り、確認の後に変わっていれば、`--force` でも書かずに `conflict`（`changed`）で止める（確認を待つ間に人が変えた内容を、見せずに消さないため）。`token` は、項目では調べる欄のいまの値、ノート・振り返りでは本文の長さと FNV-1a（64 ビット）の値（版の番号ではない。同じ本文の保存で版だけが進んでも止めないため。本文そのものは大きく、依頼の上限（4.4）を超えうるので持ち回らない。確認の後に変わったかを調べる目印で、署名ではない）。取り消したら元の記録を確認済み（`seen_at`）にする（利用者が見て扱い終えた変更を、未確認の数に残さないため。取り消さずに確認済みにするのは変更履歴の画面。7.4）。項目の欄がすでに戻したい値になっているときは、`after` と違っていても人が変えた欄に数えない（上書きするものが無い）。`after` が空の記録は取り消せない（`invalid_argument`）。ノートの `--force` は本文全体を `before` に戻すので、表示する差分は「いまの本文 → 戻す本文」で、失う内容が全部見える。

**判定の順**（コア。最初に当たったものを返す。3.2 と同じ形）

1. 記録が無い：`not_found`（利用者の保存と行き違って消えた記録も含む。アーキテクチャ設計書 4.1）。
2. 取り消しの記録、または人の記録：`invalid_argument`。
3. すでに取り消し済み：`updated: false` の成功。
4. 対象の項目が削除されている：`not_found`（項目の欄の変更。追加の記録は、すでに戻っている扱いで成功）。
5. 項目の欄の変更で、項目が持ち越し済み：`conflict`（`reason: carried_over`、`carried_to_id`・`carried_to_date` つき）。`--force` でも通さない。
6. 項目の欄の変更で、項目が取りやめた TODO（`dropped`）：`invalid_argument`（`close_reason` つき）。`--force` でも通さない。
7. 画面の保存の失敗中：`storage_unavailable`。続けて、書き戻す欄を利用者が入力中なら `conflict`（`reason: user_editing`。項目の本文・解消メモ、ノート・振り返りの守る行がある間）。アプリの停止中は画面が無いので、どちらも起きない。
8. `expect` が合わない、または人があとから変えている（`--force` なし）：`conflict`（`reason: changed`、`changed_fields`）。

**持ち越し済み・取りやめた TODO（9章の決定）**：取り消しは「画面で人ができること」だけを行う。持ち越し済みの項目と取りやめた TODO の `status`・`text`・`resolution_note`・`completed_at`・`close_reason` は、画面でも変えられない（データモデル設計書 3章 item）ので、取り消しでも変えない。エージェントの項目の記録が持つ欄は、どれもこの欄にあたるため、5・6 で拒んでも、取り消せずに残る欄は無い（持ち越した後の誤りは、持ち越し先の項目を人が直す）。一方、追加の取り消しは削除で、削除は画面でもできるので、持ち越し済みでも取りやめた TODO でも拒まない（確認の表示に、持ち越し先の項目 `carried_to_id` が残ることを出す）。

**CLI の流れ**：引数（`CHANGE_ID` の形。ULID）→ 端末か（端末でなければ何も呼ばずに `permission_denied`。終了コード 3）→ `preview_revert` → 取り消す内容の表示 → `[y/N]` → `revert_change`。
- 表示：`Change ID: operation by クライアント at 日時`、続けて取り消す内容。項目の欄は `欄: いま -> 戻す値`（人が変えた欄には `(changed after the record)`）、追加の取り消しは `This deletes the todo added on 日付: "本文"`（持ち越し先があれば残ることも）、本文は変わる行の差分（いまの本文にあって無くなる行に `-`、戻すと入る行に `+`。共通の先頭と末尾の行は除く。既定は 20 行まで。`--force` のときは、失う内容が全部見えるよう、API が返す分（200 行まで。超えたら `truncated`）をすべて出す）。
- 人が変えた欄があり `--force` が無ければ、表示のあとで `conflict`（`changed`。終了コード 5）で終わる。`--force` があれば `--force: this overwrites changes made after the record (欄).` を表示して聞く。
- 値を書かない場合（すでに取り消し済み・すでに戻っている）は、聞かずに結果を表示する（`Already reverted. Nothing changed.`、`Already back to the earlier values. The change is marked as reverted.`）。書いたら `Reverted.`（0）。取りやめたら `Cancelled. Nothing was changed.`（0）。
- 画面への知らせ：書いたとき（と、取り消し済みにしたとき）は `changed`（`by: agent`）を出す。ノートの取り込みはエージェントの変更と同じ（アーキテクチャ設計書 4.1）。

**`origin = human` にする手段（9章の決定）**：設けない。CLI は人とエージェントを確実には区別できない（2章）ので、対話端末で実行したことを理由に `human` にすると、エージェントが擬似端末で通して、自分の変更を履歴・未確認の印・取り消しの対象から外せてしまう。人が項目を足すのはアプリ（画面）の役割で、CLI で足した項目は、誰が実行しても `origin = agent` の `cli` クライアントとして記録する。

### 7.4 画面の管理の操作（設定のエージェント区分・変更履歴 S-06。2026-11-05 決定。YUN-81）

画面は、ローカル API を通さず、同じコア（`domain::agent_admin`・`domain::history`）を Tauri コマンドで呼ぶ（画面はコアと同じプロセスにあり、トークンもソケットも要らないため。アーキテクチャ設計書 4.3）。型は Rust の `src-tauri/src/types.rs` から作る `src/lib/bindings.ts`。コードは `src-tauri/src/commands.rs`（書き込みは `AppCore::admin_write`）。

| コマンド | 引数 | 返り値 | コア |
| --- | --- | --- | --- |
| `list_agents` | なし | 登録順の `[{id, name, scopes, createdAt, lastSeenAt, removable}]` | `agent_admin::list_entries` |
| `set_agent_scopes` | `client`、`allow`、`deny`、`expect`（見せた変更前のスコープ。必ず送る） | `{updated, scopes}` | `agent_admin::change_scopes` |
| `remove_agent` | `client` | なし | `agent_admin::remove_client`（下記。画面だけの口） |
| `api_status` | なし | `{state: stopped}`・`{state: listening, connections}`・`{state: unavailable, cause}` | `AppCore::api_state`（原因の文は渡さず、種類 `cause` だけ） |
| `list_changes` | `client`、`unseenOnly`、`before`、`limit` | `{changes, nextBefore}`（形は 7.2。時刻はミリ秒、項目の欄の値は文字列か空） | `history::list_changes` |
| `preview_revert` | `changeId` | 7.3 の計画（`state`、`token`、`changedSince`、`effect`） | `history::preview` |
| `revert_change` | `changeId`、`force`、`expect`（`preview_revert` の `token`。必ず送る） | `{updated, state, marked}` | `history::revert`（画面の状態 `ScreenState` を渡す） |
| `mark_changes_seen` | `ids`（画面が見せた記録） | 印を付けた件数 | `history::mark_seen`（下記。画面だけの口） |

- `list_changes` の `nextBefore` は、画面のコマンドでは 1 件多く読んで、続きがあるときだけ返す（API の `next_before` は `limit` 件ちょうど返したときに返すので、ちょうど `limit` 件で終わると空の続きを指す。画面は空の「さらに読み込む」を出さない）。200 件の上限に当たったときだけは、API と同じ規則。
- 権限の変更・登録の削除・取り消し・確認済みは、変えたら `changed`（`by: agent`）を知らせる（CLI の管理の操作と同じ経路。画面はどの `by` でも表示中のデータを読み直す）。画面の保存の失敗中には数えない。取り消しは、画面の入力中・保存の失敗中を 7.3 の判定の順どおりに見る（`ScreenState` を渡す）。
- コアのエラーは `CommandError` にする：権限の変更の `Changed` は `conflict`（`scopes_changed`）、`unknown` への `read` 以外は `invalid_argument`（`unknown_write`）、取り消しの `Changed` は `conflict`（`changed`、`afterPreview`）、`UserEditing`・`CarriedOver` は既存の `conflict`、画面の保存の失敗中は `conflict`（`save_failing`）、取りやめた TODO は `invalid_argument`（`dropped`）、取り消しの記録・読み取れない記録は `invalid_argument`（`not_agent_change`・`unsupported_record`）。
- **確認済みの印を付ける口**（`history::mark_seen(conn, ids, now)`）：渡した記録の `seen_at` を、まだ空のものだけ現在時刻にする（最初に確認した時刻を残す）。見せた記録の ID だけを渡す（一覧を読んでから印を付けるまでの間に届いた変更を、見せずに確認済みにしないため）。変更履歴の画面を開いただけでは付けない（上部のバーの件数と、整理モードの行頭の菱形が、見ていない間に消えないように）。画面が付けるのは、行ごとの「確認済みにする」と、表示している未確認すべての「確認済みにする」。取り消し（7.3）は、取り消した記録に自分で付ける。ローカル API・CLI には出さない（`seen_at` は利用者が画面で確認した時刻で、エージェントや端末から付ける理由が無いため）。
- **登録の整理**（`agent_admin::remove_client`）：登録の数の上限（4.4。64）に達すると、新しい識別子の依頼は `invalid_argument`（`TooMany`）になり、`message` は設定のエージェント区分で整理するよう案内する。整理の手段は、**変更履歴にも項目にも指されていない登録を消す**ことに決めた（行を消すのは、指す記録が無いときだけ。確かめと削除は同じトランザクション）。消せないのは、固定の `cli`・`unknown`（`invalid_argument`、`fixed_client`）と、change_log か item の `agent_client_id` に指されている登録（`conflict`、`client_in_use`。履歴の表示に名前が要る）。消した識別子が次に接続したときは、初めての識別子と同じく `read` だけで登録し直される（確かめる文に書く）。「使わない」印を付ける案は、表の版を上げることになり、印の付いた登録を 64 に数えるかという別の決まりも増えるので採らない。履歴ごと消す案も、履歴の表示と取り消しの対象を失うので採らない。理由：名乗りの乱発で上限を埋めるのは、書き込みのスコープを与えていない登録（`read` だけでは change_log も item も作れない）で、指す記録が無いため。書き込みの権限を与えて使った登録だけで 64 に達する場合の手段は設けない（そうなったら、利用者の判断が要る事項として、新しい課題にする）。登録の整理は、利用者が画面で確かめて行う操作で、エージェントや端末から呼ぶ理由が無い（管理の口を増やさない）ので、ローカル API・CLI には出さず、画面だけの口にする。
- **権限を足す確認**：画面は、権限を足すときだけ、足す権限で何ができるか（2章の表）を書いた確かめる文を、その行の下に出してから送る（CLI の `now:`・`after:` の確認に当たる。2章「確認は、CLI（対話端末）かアプリの設定画面が行い」）。外すときは、権限を狭めるだけなので確かめない。見せた権限を `expect` に付けて送り、確かめている間に別の所で変わっていたら、書かずに伝えて一覧を読み直す。

## 8. 第2フェーズで追加する操作（案。段階はロードマップ参照）

- エージェントの直接起動：`yunagi run ID --agent claude-code`
- フック連携：エージェントの作業終了時に要約を振り返りの下書きへ追記
- 変更の購読：`subscribe_changes`（MCP の通知機能を利用）

## 9. 未決事項

- エラーの `message` を利用者の言語設定（`locale`）に合わせて日英で出すか、英語固定にするか
- 書き込みの権限を与えて使った登録だけで、登録できる数の上限（4.4。64）に達したときの手段（履歴の無い登録は、設定のエージェント区分で消せる。7.4）。起きたら、利用者の判断が要る事項として扱う
