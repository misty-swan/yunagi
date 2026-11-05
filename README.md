# Yunagi

PC画面全体を「その日の仕事ノート1枚」として使う、エンジニア向けの日次業務整理アプリ。設計書は `docs/`（案内は `docs/README.md`）。

## 構成

| 場所 | 中身 |
| --- | --- |
| `crates/core/` | コア（`yunagi-core`）。ドメイン層（`domain`）とストレージ（`storage`） |
| `src-tauri/` | デスクトップアプリ（Tauri 2、`yunagi-app`）。コアを使う。画面とコアのつなぎ（Tauri コマンド・イベント。`docs/design/01-architecture.md` 4.3）。フォルダを選ぶ OS のダイアログは Tauri のプラグイン（`tauri-plugin-dialog`。権限は `capabilities/default.json` の `dialog:allow-open`）。依頼文の書き出し（Mod+Shift+A）のクリップボードへの書き込みは `src-tauri/src/clipboard.rs`（Tauri のプラグイン `tauri-plugin-clipboard-manager`。書き込みは Rust 側の `copy_handoff` だけで、画面に権限は渡さない。方式は `docs/design/01-architecture.md` 4.3）。Vim モードの IME の英数切替は `src-tauri/src/ime.rs`（macOS の入力ソース API・Windows の IMM・Linux の `fcitx5-remote` / `ibus engine`。方式は `docs/design/01-architecture.md` 3.4）。OS 全体のショートカット（Yunagi を前面に呼び出す）の登録は `src-tauri/src/global_shortcut.rs`（Tauri のプラグイン `tauri-plugin-global-shortcut`。登録は Rust 側だけで、画面に権限は渡さない。方式は同 3.5）。自動更新の確認と「更新して再起動」は `src-tauri/src/updater.rs`（Tauri のプラグイン `tauri-plugin-updater`。確認は Rust 側だけで、画面に権限は渡さない。公開鍵と配布元は `tauri.conf.json` の `plugins.updater` で、空の間は確認しない。方式は `docs/release/07-build-and-distribution.md` 5章）。窓を前面に出す部品（`bring_to_front`）は、`yunagi show`（ローカル API の `show_app`。`api/dispatch.rs` が先に名前で分け、`AppCore::request_show` → `Notice::ShowRequested` → `lib.rs` の `show_window` が窓を出して画面に `show-requested` を知らせる）と、2 つ目の起動の引き渡し（`tauri-plugin-single-instance`。`run` の最初のプラグイン）も使う（方式は同 3.5）。ローカル API の待ち受けは `src-tauri/src/api/`（`mod.rs` が待ち受けと接続の処理、`dispatch.rs` が依頼を `AppCore::api_read`・`api_write` に渡して 05 4.4 の JSON にする。方式は同 4.6） |
| `crates/api/` | ローカル API の取り決め（`yunagi-api`）。アプリ（待ち受け）と `yunagi`（接続）が共有する。通信路（UNIX ドメインソケット・Windows の名前付きパイプ。`transport`）・認証のトークン（`token`）・プロトコルの型と行の読み方（`wire`）・1 回分の接続（`client`）・操作（`get_board`・`add_todo`・`search` など）の引数を読み、コアの入口を呼び、応答とエラーを 05 の JSON にする処理（`ops`。アプリの待ち受けと直接モードの CLI・MCP が同じものを使う）。窓を前面に出す操作 `show_app` の名前・引数・返り値は `show`（データには触れず、直接モードも MCP も無いので `ops` とは別）。方式は `docs/design/01-architecture.md` 4.6、操作の中身は `docs/design/05-mcp-cli-spec.md` 4.4 |
| `crates/cli/` | コンソール用バイナリ `yunagi`（CLI・MCP サーバー。`yunagi-cli`）。コアと `yunagi-api` を使う。`src/lib.rs` が本体で、`main.rs` は呼ぶだけ（試験が同じプロセスの中で動かすため）。接続の切り替え（ローカル API か直接モードか。CLI と `yunagi mcp` が共有する）は `src/connect.rs`、引数は `src/args.rs`、テキスト出力は `src/output.rs`、`doctor` は `src/doctor.rs`、`yunagi show`（アプリを前面に出す。ローカル API だけを使い、動いていなければアプリを探して起動する。方式は `docs/design/05-mcp-cli-spec.md` 3.3）は `src/show.rs`、エラーと終了コードは `src/error.rs`、MCP サーバー（`yunagi mcp`。stdio）は `src/mcp.rs`（入出力とプロトコル。SDK は使わず自前の JSON-RPC。`Connector::call` を呼ぶだけの薄い層）とツールの定義 `src/mcp_tools.rs`、`yunagi agent setup`（Claude Code・Codex への登録。方式は `docs/design/05-mcp-cli-spec.md` 6章）は `src/agent_setup/`（`mod.rs` が流れ・確認・既定ディレクトリのずれの警告、`claude.rs` が `claude mcp add` の実行、`codex.rs` が `toml_edit` による `config.toml` の `[mcp_servers.yunagi]` だけの書き換え、`files.rs` がバックアップと壊れにくい書き込み。試験は偽の `claude` と一時フォルダで行い、利用者の設定には触れない）、`yunagi agent permissions`・`agent revert`（権限の変更と取り消し。方式は `docs/design/05-mcp-cli-spec.md` 7章）は `src/agent_admin.rs`（端末での確認と表示。確認を得た後の処理はローカル API の管理の操作に頼む）。方式は `docs/design/01-architecture.md` 4.4・4.5・4.7、出力と終了コードは `docs/design/05-mcp-cli-spec.md` 4.1。別の候補の既定ディレクトリを調べる部品（既定ディレクトリのずれの検出）はコアの `storage::dir_mismatch`。エージェントの一覧・権限の変更・登録の整理（画面だけ）はコアの `domain::agent_admin`、変更履歴の一覧・確認済みの印・取り消しはコアの `domain::history`（どちらも MCP には出さない。CLI はローカル API の `list_agents`・`set_agent_scopes`・`list_changes`・`preview_revert`・`revert_change` を使い、アプリの画面は同じコアを Tauri コマンド（`src-tauri/src/commands.rs`）で呼ぶ） |
| `install/` | インストールスクリプト。macOS・Linux 用の `install.sh`（POSIX sh）と Windows 用の `install.ps1`、その試験 `install.test.ts`（vitest。`pnpm check:web` に含まれる。一時フォルダの `HOME`・偽の `uname`・偽の配布元（`file://`）で動かし、利用者の環境には触れない。`install.ps1` の試験は `pwsh` があるときだけ走る）。動作は `docs/release/07-build-and-distribution.md` 4章 |
| `scripts/docs/` | 利用者ガイド（`docs/user/ja/guide.md`・`docs/user/en/guide.md`）の試験 `guide.test.ts`（`pnpm check:web` に含まれる）。日英の見出し・コードの囲み・表・キーの並びがそろっていることと、ガイドのキー・コマンド・画面の文言がコード（`src/lib/keymap.ts`・`crates/cli/src/args.rs`・`src/lib/i18n*.ts`）と合っていることを確かめる。決まりは `docs/release/08-test-plan.md` 4.12 |
| `scripts/release/` | 配布用 CI（`.github/workflows/release.yml`）が使うスクリプトと設定（TypeScript。Node.js がそのまま動かす）とその試験（`*.test.ts`。`pnpm check:web` に含まれる）。既知の脆弱性の検査と結果の分け方（`audit.ts`）・許可リストとその期限の検査（`audit-allowlist.json`・`allowlist.ts`・`check-allowlist.ts`）・許可外ライセンスの検出と `THIRD_PARTY_LICENSES.md` の生成（`licenses.ts`・`license-policy.json`。Rust の依存の収集は直下の `about.toml`）・タグと版の一致の確認（`version.ts`）・ビルドの組み合わせ（`matrix.ts`）・配布物の名前と中身をそろえて `.sha256` を添える（`package.ts`）。動作は `docs/release/07-build-and-distribution.md` 3章。配布用のビルドの Tauri の設定の重ね書きは `src-tauri/release/` |
| `src/` | 画面（TypeScript ＋ Svelte 5、Vite）。コアの呼び出し口は `src/lib/core.ts`。画面の外枠（サイドバー・上部のバー・凪メーター・キーの案内）は `src/shell/`、キーの割当（操作の一覧・初期割当・割当の文字列・衝突の検査）は `src/lib/keymap.ts`、キーの押下から操作を読むのは `src/lib/keys.ts`（外枠・ノート）と `src/lib/board_keys.ts`（整理モード）、整理モードの本文（TODO・障害・振り返りの3領域）は `src/board/`（一覧の状態と操作は `src/lib/board.svelte.ts`）、ノートモードの本文（ノートのエディタと Outline・Sent）は `src/note/`（振り返りと共通のエディタの動きは `src/lib/doc_editor.ts`、エディタの Vim のキー（`@replit/codemirror-vim`。ノートの `<Space>t` など）は `src/lib/editor_vim.ts`、そのモードの表示は `src/shell/VimBadge.svelte`、挿入モードの出入りに合わせた IME の切替の呼び出しと切替ができる環境かの問い合わせは `src/lib/ime.svelte.ts`、時刻付き見出しなどの本文の扱いは `src/lib/note.ts`、送信済みの印と行送りで送る行は `src/lib/note_marks.ts`、等幅の書体の読み込みは `src/lib/fonts.svelte.ts`）、入力中の状態の知らせは `src/lib/ui_report.svelte.ts`、ふりかえりの画面は `src/archive/`（状態と操作は `src/lib/archive.svelte.ts`、GOOD / STUCK などの取り出しは `src/lib/archive_text.ts`、文言は `src/lib/i18n_archive.ts`）、持ち越しの画面は `src/carryover/`（状態と操作は `src/lib/carryover.svelte.ts`）、検索パレットは `src/palette/`（状態と検索は `src/lib/palette.svelte.ts`、日付ジャンプの読み方は `src/lib/date_query.ts`、一致した所の切り出しは `src/lib/search_match.ts`）、データベースを開くまでの起動時の画面は `src/startup/`（操作は `src/lib/startup.svelte.ts`、入れ直し方と問い合わせ先は `src/lib/support.ts`）、エージェント変更履歴（S-06）は `src/agents/`（画面は `Agents.svelte`、エージェントの一覧と権限の切り替えは設定のエージェント区分と共通の `AgentList.svelte`。一覧・絞り込み・確認済み・取り消しの状態と操作は `src/lib/agents.svelte.ts`、権限の切り替え・登録の整理・ローカル API の状態は `src/lib/agent_admin.svelte.ts`、文言は `src/lib/i18n_agents.ts`。Tauri コマンドは `docs/design/05-mcp-cli-spec.md` 7.4）、自動更新の通知（サイドバー下端・上部のバーの「New version」）は `src/shell/UpdateNotice.svelte`（状態と操作は `src/lib/update.svelte.ts`、文言は `src/lib/i18n_update.ts`）、設定画面は `src/settings/`（設定の読み込みと保存は `src/lib/settings.svelte.ts`、開閉と区分は `src/lib/settings_panel.svelte.ts`、ショートカット割当のキーの取り込みは `src/lib/shortcuts_edit.svelte.ts`、OS 全体のショートカットの登録のし直しと登録の状態は `src/lib/global_shortcut.svelte.ts`、データ区分の操作は `src/lib/settings_data.svelte.ts`、画面が使う設定の値は `src/lib/prefs.svelte.ts`、OS のフォルダ選択は `src/lib/folder.ts`、バックアップの表示は `src/lib/backups.ts`、文言は `src/lib/i18n_settings.ts`）、色のトークンと文字の土台は `src/styles/`、同梱するフォント（配布元のファイルと許諾の表記。フォントごとのフォルダ）は `src/assets/fonts/`、画面の文言（日本語・英語）は `src/lib/i18n.ts` |

Rust の4つのクレートは、直下の `Cargo.toml` で1つのワークスペースにまとめている。依存の版は `Cargo.lock` と `pnpm-lock.yaml` で固定する。

### 画面と共有する型

画面とコアが受け渡す型は Rust（`src-tauri/src/types.rs`）に書き、TypeScript の定義 `src/lib/bindings.ts` は ts-rs でそこから作る（手で書き換えない）。Rust の型やイベントの名前を変えたら、次で作り直す。

```sh
UPDATE_BINDINGS=1 cargo test -p yunagi-app --lib bindings   # bindings.ts を Rust の型から作り直す
```

作り直さないまま `pnpm check` を走らせると、`bindings.ts` が Rust の型と食い違っているとして失敗する。

画面とコアの結合テスト（`src-tauri/tests/bridge.rs`）は、窓を開かずに、画面からと同じ経路（IPC）でコマンドを呼び、結果とイベントを確かめる。既定ディレクトリを一時フォルダにするため、環境変数を設定してから1つのスレッドで順に走らせる（`pnpm check:rust` に含まれる）。ローカル API の結合テスト（`src-tauri/tests/api.rs`。テスト計画書 4.5）は、実際に UNIX ドメインソケットで待ち受け、`yunagi-api` のクライアントで依頼を送って確かめる（同じく環境変数を設定して順に走らせる。UNIX 系だけで走る）。強制終了の試験（`src-tauri/tests/crash.rs`。テスト計画書 4.1）も同じく窓を開かず、コアを子プロセスで動かして保存の途中で強制終了し、データベースを開き直して入力が残っていることを確かめる（十数秒かかる）。CLI の結合テスト（`crates/cli/tests/cli.rs`。テスト計画書 4.5）は、一時フォルダの既定ディレクトリで `yunagi_cli::run` を同じプロセスの中で動かし、直接モード・接続の手順（偽のアプリを含む）・`doctor`・MCP サーバー（標準入力に行を渡して応答を確かめる）を確かめる（同じく環境変数を設定して順に走らせる。UNIX 系だけで走る）。アプリ経由の CLI・MCP サーバーは `src-tauri/tests/api.rs` で確かめる。

## 必要なもの

- Rust 1.99.1（`rust-toolchain.toml` で固定。rustup を使っていれば自動で入る。`Cargo.toml` の `rust-version = "1.99"` はコンパイルできる最低の版の宣言で、検査は固定した 1.99.1 で行う）、Node.js 26.8.2（`.node-version` で固定）、pnpm 12（`package.json` の `packageManager`）
- Tauri 2 の動作に必要な OS のライブラリ（<https://v2.tauri.app/start/prerequisites/>）

## よく使うコマンド

```sh
pnpm install --frozen-lockfile   # ロックファイルどおりに依存を入れる
pnpm check                       # Rust の書式の検査・単体テスト・clippy・文書の生成・画面の型チェック・画面のテストをまとめて走らせる
pnpm tauri dev                   # 開発サーバーとアプリを起動する（ウィンドウが開く）
cargo run -p yunagi-cli -- board # コンソール用バイナリ yunagi を動かす（今日のボードを表示する。`--` の後ろが yunagi の引数）
```

`pnpm check` は次の7つを順に走らせ、1つでも失敗すると止まる。個別にも走らせられる。

- `pnpm check:fmt`：Rust のコードの書式が rustfmt（Rust の標準の整形ツール）の形にそろっているかを確かめる。`cargo fmt --all --check`（ファイルは書き換えない。ずれていたら `cargo fmt --all` で直す）
- `pnpm check:rust`：Rust の4つのクレート全体の単体テスト。`cargo test --workspace --locked`（`Cargo.lock` と食い違うと失敗する）。デスクトップアプリ（`yunagi-app`）もテスト用に実際にビルドするので、Tauri と WebView のライブラリを含む初回は数分かかる。2回目以降は変わった所だけビルドするので速い
- `pnpm check:clippy`：Rust の4つのクレート全体（テストのコードを含む）を clippy（Rust のコードの書き方の問題を指摘する検査）にかける。`cargo clippy --workspace --all-targets --locked -- -D warnings`（警告が1つでもあれば失敗する。型の確認までで、実行ファイルは作らない）
- `pnpm check:doc`：Rust の4つのクレートのコード中の説明（doc コメント）から文書を作り、説明の中のリンク切れを見つける。`cargo doc --workspace --no-deps --locked --config "build.rustdocflags=['-D','warnings']"` を、`--document-private-items` ありとなしで2回走らせる。1回目は公開していない項目の説明も作り、その中のリンク切れを見つける（2回目の設定では、公開していない項目の説明は検査されない）。2回目は公開している項目の説明だけを作り、公開していない項目へのリンクを見つける（この検査は1回目の設定では働かない）。`target/doc` の目次は2回目の公開の版になる（1回目で作った非公開の項目のページのファイルは残るが、目次からは辿れない）。2回の設定が違うので、変更がなくても毎回作り直す（数秒）。警告が1つでもあれば失敗する。依存ライブラリの文書は作らない。説明の警告（リンク切れ・HTML の誤り・脚注の誤りなど、rustdoc が既定で警告にするもの。`rustdoc -W help` で `warn` のもの）は、直下の `Cargo.toml` の `[workspace.lints.rustdoc]` で失敗にしており、4つのクレートが `[lints] workspace = true` で使う。こちらは環境変数 `RUSTDOCFLAGS`・`CARGO_ENCODED_RUSTDOCFLAGS` や、cargo の設定ファイル（`~/.cargo/config.toml` など）の `rustdocflags` を設定していても効く（その中で `-A` などで明示的に許したものを除く）。`--config` の `-D warnings` は、それ以外の警告も失敗にするための重ねがけで、`RUSTDOCFLAGS` などがあると cargo がそちらを優先して効かなくなる（空の値でも同じ）。`--config` で渡すのは、環境変数を使う書き方と違って Windows でも同じ1行で動くため。Rust の版を上げて、既定で警告になるものが増えたときは `[workspace.lints.rustdoc]` に足す
- `pnpm check:types`：画面のコードの型チェック（`svelte-check`）と、設定ファイル（`vite.config.ts`・`svelte.config.js`）の型チェック（`tsc -p tsconfig.node.json`）
- `pnpm check:web`：`vitest run`（`src/`・`install/`・`scripts/` の試験。`scripts/docs/guide.test.ts` は利用者ガイド（日英）の構成がそろっていることと、キー・コマンド・画面の文言がコードと合っていることを確かめる。`docs/release/08-test-plan.md` 4.12）
- `pnpm check:allowlist`：既知の脆弱性の許可リスト（`scripts/release/audit-allowlist.json`）の期限が切れていないことと、`.cargo/audit.toml`・`pnpm-workspace.yaml` の ID との一致を、今日の日付で確かめる（`node scripts/release/check-allowlist.ts`）。期限の翌日から失敗する。決まりは `docs/release/07-build-and-distribution.md` 3.1

GitHub Actions の開発用 CI（`.github/workflows/ci.yml`）は、PR と main への push のたびに、Linux（Ubuntu）で同じ `pnpm check` を走らせる。macOS・Windows では、Rust のテスト・clippy・`cargo doc` だけを走らせる。ただし費用のため、リポジトリが公開のときと手動実行（`gh workflow run CI --ref <ブランチ>`）のときだけで、非公開の間は自動では回らない（範囲・費用・手順は `docs/release/08-test-plan.md` 2章）。配布用 CI の PR の試運転のビルドも同じ。手元と CI で検査の中身がずれないよう、検査を足すときは `package.json` の `check` に足す。Rust と Node.js の版も、手元と同じ `rust-toolchain.toml`・`.node-version` から読む。配布用のビルド（`docs/release/07-build-and-distribution.md` 3章）とは別のもの。配布用の CI は `.github/workflows/release.yml`：タグ `v<ver>`（`<ver>` は直下の `Cargo.toml` の版と同じ）の push で、依存の既知脆弱性・許可外ライセンス・フォントの検査と、6 組（OS × アーキテクチャ）のビルドを行う。このファイルや `src-tauri/release/`・`scripts/release/` を変える PR では、タグなしの試運転も走る。使うスクリプトと設定は `scripts/release/`（試験は同じ場所の `*.test.ts`）、`about.toml`（cargo-about）、`src-tauri/release/`（配布用のビルドの設定）。署名と Release の作成は未実装（Linear の YUN-101）。ワークフローの `uses:` の action は、タグではなくコミットの SHA で固定し、末尾に版を書く。更新は Dependabot（`.github/dependabot.yml`）の PR で行う（決まりは同じ 3章の「action の固定」）。

版は直下の `Cargo.toml` の `[workspace.package]` の `version` 1か所にだけ書く。4つのクレートはこれを使い、デスクトップアプリも `src-tauri/tauri.conf.json` に版を書かないことで、Tauri がアプリのクレートの版（＝ワークスペースの版）を使う。`package.json` は公開しないので版を書かない。食い違わないことは `yunagi-cli` の単体テストで確かめている。

画面に読み込んでよいもの（CSP）は `src-tauri/tauri.conf.json` の `app.security` に書く。`csp` は配布するアプリ用、`devCsp` は `pnpm tauri dev` 用で、開発サーバーとの自動更新（HMR）の通信（`ws:`）だけを足している。

型チェックの設定は2つに分けている。`tsconfig.json` は画面（`src/`。ブラウザで動く）用、`tsconfig.node.json` は設定ファイル（Node.js で動く）用。

`experiments/`（技術選定の試作）と `prototype/`（画面の見本）は本番のコードではなく、上のコマンドの対象にも入れていない。

## コードの書き方

### エラーの表示（Rust）

- エラーの型の Display（`to_string()` で得る文）には、その層で何が失敗したか（何をしようとしたか・どのファイルか）だけを書き、原因のエラーの文は含めない。原因は `source()` で返す。両方に書くと、原因をたどって表示したときに同じ文が二重に出るため（2026-11-03 決定）。
- 別のエラーを包むだけの種類（`StorageError::Lock`・`BackupError::Storage` など）は、Display も `source()` も中身のものをそのまま返す（透過）。そのため包んだ型は `source()` をたどっても出てこない。失敗の種類で振る舞いを変えるとき（ローカル API の接続の失敗の種類で、直接モードに切り替えるかを決めるなど）は、`source()` をたどらず、エラーの種類を `match` で見分ける。
- 原因が2つある種類（`StorageError::RestoreFailed` など）は、主な原因を `source()` で返し、もう一方は原因まで含めて Display に書く。主な原因が後ろにつながったとき、どの失敗の原因かが分かるよう、主な失敗を文の最後に書く。
- 利用者やログに1行で出すときは、`yunagi_core::error::ErrorChain` で原因まで `: ` でつなぐ。rusqlite など外部のエラーには Display と `source()` の両方に同じ文を出すものがあるので、`ErrorChain` は、前の文と同じ原因の文や、前の文の先頭・末尾に区切って付いている原因の文を省く。
- MCP・CLI のエラーの `message` と画面のエラー表示の文は、Display をそのまま使わない（`docs/design/05-mcp-cli-spec.md` 4.3、`docs/design/03-ui-spec.md` 3章の保存の失敗の表示）。
