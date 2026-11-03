# Yunagi

PC画面全体を「その日の仕事ノート1枚」として使う、エンジニア向けの日次業務整理アプリ。設計書は `docs/`（案内は `docs/README.md`）。

## 構成

| 場所 | 中身 |
| --- | --- |
| `crates/core/` | コア（`yunagi-core`）。ドメイン層（`domain`）とストレージ（`storage`） |
| `src-tauri/` | デスクトップアプリ（Tauri 2、`yunagi-app`）。コアを使う |
| `crates/cli/` | コンソール用バイナリ `yunagi`（CLI・MCP サーバー）。コアを使う |
| `src/` | 画面（TypeScript ＋ Svelte 5、Vite） |

Rust の3つのクレートは、直下の `Cargo.toml` で1つのワークスペースにまとめている。依存の版は `Cargo.lock` と `pnpm-lock.yaml` で固定する。

## 必要なもの

- Rust 1.99.1（`rust-toolchain.toml` で固定。rustup を使っていれば自動で入る。`Cargo.toml` の `rust-version = "1.99"` はコンパイルできる最低の版の宣言で、検査は固定した 1.99.1 で行う）、Node.js 26.8.2（`.node-version` で固定）、pnpm 12（`package.json` の `packageManager`）
- Tauri 2 の動作に必要な OS のライブラリ（<https://v2.tauri.app/start/prerequisites/>）

## よく使うコマンド

```sh
pnpm install --frozen-lockfile   # ロックファイルどおりに依存を入れる
pnpm check                       # Rust の書式の検査・単体テスト・clippy・画面の型チェック・画面のテストをまとめて走らせる
pnpm tauri dev                   # 開発サーバーとアプリを起動する（ウィンドウが開く）
cargo run -p yunagi-cli          # コンソール用バイナリ yunagi を動かす
```

`pnpm check` は次の5つを順に走らせ、1つでも失敗すると止まる。個別にも走らせられる。

- `pnpm check:fmt`：Rust のコードの書式が rustfmt（Rust の標準の整形ツール）の形にそろっているかを確かめる。`cargo fmt --all --check`（ファイルは書き換えない。ずれていたら `cargo fmt --all` で直す）
- `pnpm check:rust`：Rust の3つのクレート全体の単体テスト。`cargo test --workspace --locked`（`Cargo.lock` と食い違うと失敗する）。デスクトップアプリ（`yunagi-app`）もテスト用に実際にビルドするので、Tauri と WebView のライブラリを含む初回は数分かかる。2回目以降は変わった所だけビルドするので速い
- `pnpm check:clippy`：Rust の3つのクレート全体（テストのコードを含む）を clippy（Rust のコードの書き方の問題を指摘する検査）にかける。`cargo clippy --workspace --all-targets --locked -- -D warnings`（警告が1つでもあれば失敗する。型の確認までで、実行ファイルは作らない）
- `pnpm check:types`：画面のコードの型チェック（`svelte-check`）と、設定ファイル（`vite.config.ts`・`svelte.config.js`）の型チェック（`tsc -p tsconfig.node.json`）
- `pnpm check:web`：`vitest run`

GitHub Actions の開発用 CI（`.github/workflows/ci.yml`）は、PR と main への push のたびに、Linux（Ubuntu）で同じ `pnpm check` を走らせる。手元と CI で検査の中身がずれないよう、検査を足すときは `package.json` の `check` に足す。Rust と Node.js の版も、手元と同じ `rust-toolchain.toml`・`.node-version` から読む。配布用のビルド（`docs/release/07-build-and-distribution.md` 3章）とは別のもの。

版は直下の `Cargo.toml` の `[workspace.package]` の `version` 1か所にだけ書く。3つのクレートはこれを使い、デスクトップアプリも `src-tauri/tauri.conf.json` に版を書かないことで、Tauri がアプリのクレートの版（＝ワークスペースの版）を使う。`package.json` は公開しないので版を書かない。食い違わないことは `yunagi-cli` の単体テストで確かめている。

画面に読み込んでよいもの（CSP）は `src-tauri/tauri.conf.json` の `app.security` に書く。`csp` は配布するアプリ用、`devCsp` は `pnpm tauri dev` 用で、開発サーバーとの自動更新（HMR）の通信（`ws:`）だけを足している。

型チェックの設定は2つに分けている。`tsconfig.json` は画面（`src/`。ブラウザで動く）用、`tsconfig.node.json` は設定ファイル（Node.js で動く）用。

`experiments/`（技術選定の試作）と `prototype/`（画面の見本）は本番のコードではなく、上のコマンドの対象にも入れていない。

## コードの書き方

### エラーの表示（Rust）

- エラーの型の Display（`to_string()` で得る文）には、その層で何が失敗したか（何をしようとしたか・どのファイルか）だけを書き、原因のエラーの文は含めない。原因は `source()` で返す。両方に書くと、原因をたどって表示したときに同じ文が二重に出るため（2026-11-03 決定）。
- 別のエラーを包むだけの種類（`StorageError::Lock`・`BackupError::Storage` など）は、Display も `source()` も中身のものをそのまま返す（透過）。
- 原因が2つある種類（`StorageError::RestoreFailed` など）は、主な原因を `source()` で返し、もう一方は原因まで含めて Display に書く。
- 利用者やログに1行で出すときは、`yunagi_core::storage::ErrorChain` で原因まで `: ` でつなぐ。rusqlite など外部のエラーには Display と `source()` の両方に同じ文を出すものがあるので、`ErrorChain` は前の文に含まれている文を省く。
- MCP・CLI のエラーの `message`（`docs/design/05-mcp-cli-spec.md` 4.3）や画面のエラー表示（`docs/design/03-ui-spec.md` の保存の失敗の原因の表示など）の文は、Display をそのまま使わず、エラーの種類から作る。原因の文は、ログや詳細の表示に `ErrorChain` で添える。
