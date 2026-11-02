# YUN-2 フロントエンドの UI フレームワーク比較（Svelte / SolidJS）

> **比較用の使い捨てです。本番に流用しません。**
> ここにあるコードは、Svelte と SolidJS のどちらを採用するかを決める材料を集めるためだけのものです。
> 本番の実装は、決定（YUN-3）と設計書（`docs/`）に従ってゼロから書きます。
> 仕様の正本は `docs/` で、ここに書いたことは仕様ではありません。

結果と推薦は [RESULTS.md](RESULTS.md) にあります。

## 中身

| フォルダ | 内容 |
| --- | --- |
| `svelte/` | Svelte 5 ＋ Vite ＋ Tauri 2 のアプリ |
| `solid/` | SolidJS 1.9 ＋ Vite ＋ Tauri 2 のアプリ |
| `shared/` | 両版で**まったく同じ**に使うファイル（IME 判定、CodeMirror の生成、計測コード、スタイル） |
| `scripts/` | 共通ファイルのコピー、ビルドの計時、各種計測 |
| `results/` | 計測の生データ（`results/raw/` はコミットしない） |

両版の画面は同じです。

- 左：TODO の入力欄と一覧。Enter で追加（IME の変換中の Enter では追加しない。`docs/design/04-interaction-spec.md` 3章の規則）。チェックで完了、「すべて消す」ボタン。
- 左下：「Event log」。入力欄とエディタで起きたキー・変換のイベントと、IME 判定の結果を表示する（実機確認の手がかり用）。
- 右：CodeMirror 6 のエディタ1つ（Markdown）。

違うのは `svelte/src/App.svelte`・`svelte/src/main.ts` と `solid/src/App.tsx`・`solid/src/main.tsx`（と設定ファイル）だけです。`src/shared/` は `shared/` のコピーで、直接は編集しません。

## 使い方

前提：Node.js と pnpm、mise で入れた Rust（rust@1.99.1）、webkit2gtk-4.1。

```sh
cd experiments/yun-2-frontend

# 共通ファイルを両版にコピーする（shared/ を変えたときだけ）
./scripts/sync-shared.sh

# 依存パッケージを入れる（初回だけ。各版の node_modules/ にダウンロードされる）
(cd svelte && pnpm install) && (cd solid && pnpm install)

# 開発用に起動する（ファイルを保存すると画面が自動で更新される）
cd svelte && mise exec rust -- pnpm tauri dev     # Solid 版は cd solid

# 本番ビルド（時間がかかる。初回は数分）。実行ファイルは src-tauri/target/release/ にできる
./scripts/build-timed.sh clean

# 計測（Hyprland の見えない特別ワークスペース special:yunagi-bench にウィンドウを開いて閉じる。
# 画面には出ない。見えない場所では描画が起きないので、起動は「画面の組み上げ完了」までを測る）
python3 scripts/measure.py startup --runs 10
python3 scripts/measure.py bench --runs 3
python3 scripts/static_metrics.py
```
