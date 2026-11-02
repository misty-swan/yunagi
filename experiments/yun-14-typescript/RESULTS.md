# YUN-14 svelte-check と TypeScript の版の確認

型チェックの道具 `svelte-check` が、どの版の TypeScript と組み合わせて動くかを確かめた記録（2026-11-02）。設計書の結論は `docs/design/01-architecture.md` 3.2。

## 1. 構成

`src/` に、型エラーをわざと1つずつ入れた `.svelte` と `.ts` を置いた。どちらも見つけられれば合格とする。共通の版は Svelte 5.57.3、svelte-check 4.7.8、@tsconfig/svelte 5.0.8。

## 2. やり直し方

組み合わせごとに、このフォルダを新しい場所へコピーしてから実行する。同じフォルダで続けて入れると、前の組み合わせの依存が残る。各コマンドは、依存を入れてから型チェックを走らせ、見つかったエラーを表示する。入れた依存（`node_modules/` など）は `.gitignore` で記録の対象から外している。

```sh
# A：TypeScript 6 だけ
npm i -D typescript@6.0.3 svelte@5.57.3 svelte-check@4.7.8 @tsconfig/svelte@5.0.8
npx svelte-check --tsconfig ./tsconfig.json

# B：TypeScript 6 と 7 を両方入れ、--tsgo を付ける
#   7 系を入れる別名 @typescript/native は、C で svelte-check 自身が表示した指示のとおり
npm i -D typescript@6.0.3 @typescript/native@npm:typescript@7.0.2 svelte@5.57.3 svelte-check@4.7.8 @tsconfig/svelte@5.0.8
npx svelte-check --tsconfig ./tsconfig.json --tsgo

# C：TypeScript 7 だけ（版の食い違いの検査を外して入れる）
npm i -D --legacy-peer-deps typescript@7.0.2 svelte@5.57.3 svelte-check@4.7.8 @tsconfig/svelte@5.0.8
npx svelte-check --tsconfig ./tsconfig.json
```

合格のときの出力は「エラー 2 件、警告 1 件」になる。警告は `App.svelte` の `state_referenced_locally` で、型エラーを作るために `$state` の値を直接読んでいることによる。

## 3. 結果

| 組み合わせ | 結果 |
| --- | --- |
| A：TypeScript 6.0.3 だけ | 2件とも見つけた（0.9 秒） |
| B：6.0.3 と 7.0.2、`--tsgo` | 2件とも見つけた（1.2 秒）。変換したファイルをディスクに書き出す試験的な方式 |
| C：7.0.2 だけ | svelte-check が起動時に止まる。「6 系と 7 系の両方を入れ、`--tsgo` を付けよ」と表示 |

TypeScript 5 系は試していない。svelte-check 4.7.8 が依存の宣言（peerDependencies）で対応を示しているのは `^5.0.0 || ^6.0.0`。

2 ファイルだけの計測なので、速さの比較としては参考にならない。
