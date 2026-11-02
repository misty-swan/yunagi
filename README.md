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

- Rust 1.99 以上、Node.js、pnpm 12
- Tauri 2 の動作に必要な OS のライブラリ（<https://v2.tauri.app/start/prerequisites/>）

## よく使うコマンド

```sh
pnpm install --frozen-lockfile   # ロックファイルどおりに依存を入れる
pnpm check                       # コアの単体テスト・画面の型チェック・画面のテストをまとめて走らせる
pnpm tauri dev                   # 開発サーバーとアプリを起動する（ウィンドウが開く）
cargo run -p yunagi-cli          # コンソール用バイナリ yunagi を動かす
```

`pnpm check` は次の3つを順に走らせ、1つでも失敗すると止まる。個別にも走らせられる。

- `pnpm check:rust`：コア（`yunagi-core`）とコンソール用バイナリ（`yunagi-cli`）の単体テスト。`cargo test -p yunagi-core -p yunagi-cli --locked`（`Cargo.lock` と食い違うと失敗する）
- `pnpm check:types`：画面のコードの型チェック（`svelte-check`）と、設定ファイル（`vite.config.ts`・`svelte.config.js`）の型チェック（`tsc -p tsconfig.node.json`）
- `pnpm check:web`：`vitest run`

デスクトップアプリ（`yunagi-app`）は Tauri と WebView のライブラリを含み、ビルドに時間がかかるため `pnpm check` には入れていない。ビルドできることは `pnpm check:app`（`cargo build -p yunagi-app --locked`）で確かめる。

型チェックの設定は2つに分けている。`tsconfig.json` は画面（`src/`。ブラウザで動く）用、`tsconfig.node.json` は設定ファイル（Node.js で動く）用。

`experiments/`（技術選定の試作）と `prototype/`（画面の見本）は本番のコードではなく、上のコマンドの対象にも入れていない。
