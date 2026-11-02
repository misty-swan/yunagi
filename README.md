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

- Rust 1.99.1（`rust-toolchain.toml` で固定。rustup を使っていれば自動で入る）、Node.js 26（`.node-version`）、pnpm 12（`package.json` の `packageManager`）
- Tauri 2 の動作に必要な OS のライブラリ（<https://v2.tauri.app/start/prerequisites/>）

## よく使うコマンド

```sh
pnpm install --frozen-lockfile   # ロックファイルどおりに依存を入れる
pnpm check                       # Rust の単体テスト・clippy・画面の型チェック・画面のテストをまとめて走らせる
pnpm tauri dev                   # 開発サーバーとアプリを起動する（ウィンドウが開く）
cargo run -p yunagi-cli          # コンソール用バイナリ yunagi を動かす
```

`pnpm check` は次の4つを順に走らせ、1つでも失敗すると止まる。個別にも走らせられる。

- `pnpm check:rust`：Rust の3つのクレート全体の単体テスト。`cargo test --workspace --locked`（`Cargo.lock` と食い違うと失敗する）
- `pnpm check:clippy`：Rust の3つのクレート全体（テストのコードを含む）を clippy（Rust のコードの書き方の問題を指摘する検査）にかける。`cargo clippy --workspace --all-targets --locked -- -D warnings`（警告が1つでもあれば失敗する。デスクトップアプリ（`yunagi-app`）がコンパイルできるかもこれで確かめる。Tauri と WebView のライブラリを含むため初回は1分ほどかかるが、2回目以降は変わった所だけ確かめるので速い）
- `pnpm check:types`：画面のコードの型チェック（`svelte-check`）と、設定ファイル（`vite.config.ts`・`svelte.config.js`）の型チェック（`tsc -p tsconfig.node.json`）
- `pnpm check:web`：`vitest run`

GitHub Actions の開発用 CI（`.github/workflows/ci.yml`）は、PR と main への push のたびに、Linux（Ubuntu）で同じ `pnpm check` を走らせる。手元と CI で検査の中身がずれないよう、検査を足すときは `package.json` の `check` に足す。Rust と Node.js の版も、手元と同じ `rust-toolchain.toml`・`.node-version` から読む。配布用のビルド（`docs/release/07-build-and-distribution.md` 3章）とは別のもの。

版は直下の `Cargo.toml` の `[workspace.package]` の `version` 1か所にだけ書く。3つのクレートはこれを使い、デスクトップアプリも `src-tauri/tauri.conf.json` に版を書かないことで、Tauri がアプリのクレートの版（＝ワークスペースの版）を使う。`package.json` は公開しないので版を書かない。食い違わないことは `yunagi-cli` の単体テストで確かめている。

画面に読み込んでよいもの（CSP）は `src-tauri/tauri.conf.json` の `app.security` に書く。`csp` は配布するアプリ用、`devCsp` は `pnpm tauri dev` 用で、開発サーバーとの自動更新（HMR）の通信（`ws:`）だけを足している。

型チェックの設定は2つに分けている。`tsconfig.json` は画面（`src/`。ブラウザで動く）用、`tsconfig.node.json` は設定ファイル（Node.js で動く）用。

`experiments/`（技術選定の試作）と `prototype/`（画面の見本）は本番のコードではなく、上のコマンドの対象にも入れていない。
