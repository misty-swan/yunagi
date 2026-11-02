// 「コア」（Rust 側のデータの持ち主）の模擬。Svelte 版・Solid 版で同じファイルを使う。
// docs/design/01-architecture.md 4章 6：エージェントが変更したら、画面はコアからデータを読み込み直す。
// 本物のコアの代わりに、メモリ上の配列を持つだけ。読み込み直し（load）は毎回、新しいオブジェクトの配列を返す。

export type Item = { id: string; text: string };

let seq = 0;
export const newItemId = () => `i${++seq}`;

const initial = ['設計書を読む', 'PR を確認する', 'テストを書く', 'Linear の課題を整理する', '振り返りを書く'];

function createCore() {
  let rows: Item[] = initial.map((text) => ({ id: newItemId(), text }));
  let agentVersion = 0;
  let agentCursor = 0;
  return {
    /** 読み込み直し。ID は同じで、中身は新しいオブジェクト */
    load(): Item[] {
      return rows.map((r) => ({ id: r.id, text: r.text }));
    },
    /** 画面での変更を保存する（自動保存の模擬） */
    save(items: readonly Item[]) {
      rows = items.map((r) => ({ id: r.id, text: r.text }));
    },
    /** エージェントの変更を模して、avoidId 以外の項目を1つ書き換える。書き換えた項目の id を返す */
    agentEdit(avoidId: string | null): string | null {
      const candidates = rows.filter((r) => r.id !== avoidId);
      if (!candidates.length) return null;
      const r = candidates[agentCursor++ % candidates.length];
      r.text = `${r.text.replace(/ ◆\d+$/, '')} ◆${++agentVersion}`;
      return r.id;
    },
  };
}

export const core = createCore();
// 自動チェック（listcheck.ts）から触れるようにする
(window as unknown as { __yun2Core: typeof core }).__yun2Core = core;
