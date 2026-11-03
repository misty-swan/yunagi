// 検索パレット（S-04。画面設計書 6章）の状態：入力・検索・コマンドの一覧。見た目は src/palette/。
//
// 1文字ずつ打っても画面が止まらないよう（テスト計画書 5章「入力応答」）：
// - コアへの検索は、打ち止めてから `SEARCH_DELAY_MS` 後に1回だけ送る。IME の変換中は送らない。
// - 後から送った検索の答えだけを使う（答えの届く順が入れ替わっても、古い答えで上書きしない）。
// - 結果の行の切り出し（本文の一部と一致の位置）は、答えが届いたときに1回だけ求め、打つたびには求めない。
//   答えが届くまでは、前の答えを、その答えの検索語の強調のまま出しておく（打つたびに一覧が消えないように）。

import type { Id, ItemKind, LogicalDate, SearchHit } from "./bindings";
import { core } from "./core";
import { parseDateQuery } from "./date_query";
import type { Messages } from "./i18n";
import { keyLabel, type ShellAction } from "./keys";
import type { Layout } from "./prefs.svelte";
import { searchTerm, snippetOf, type Snippet } from "./search_match";
import type { Mode } from "./shell.svelte";

/** 打ち止めてから検索を送るまで（ミリ秒）。 */
export const SEARCH_DELAY_MS = 120;
/** 1回の検索で出す件数（日付の新しい順）。 */
export const SEARCH_LIMIT = 50;

/** 結果の行に出す形。 */
export interface HitView {
  hit: SearchHit;
  /** 本文の一部（一致した部分を強調する）。 */
  body: Snippet;
  /** 解消メモで一致した障害の、解消メモの一部（本文で一致したときは `null`）。 */
  note: Snippet | null;
}

/**
 * 検索結果から移る、整理モードの中の先：項目（その行の本文の終わりにカーソル）か、振り返りの中の
 * 検索語 `find` と一致した所。`n` は選ぶたびに変わる番号（同じ結果をもう一度選んでも移る）。
 */
export type BoardTarget = { date: LogicalDate; n: number } & ({ itemId: Id } | { find: string });

/** 検索結果から移る、ノートの中の検索語 `find` と一致した所。 */
export interface NoteFind {
  date: LogicalDate;
  find: string;
}

/** パレットで選んだもの（App が行う）。`query` は結果を探した検索語（移った先で一致した所を探す）。 */
export type PaletteChoice =
  | { kind: "date"; date: LogicalDate }
  | { kind: "command"; id: PaletteCommandId }
  | { kind: "hit"; view: HitView; query: string };

/** 結果を行の形にする（検索語 `query` で一致した所を切り出す）。 */
export function hitView(hit: SearchHit, query: string): HitView {
  // 本文の一部は、一致した所があればそれを含む
  const body = snippetOf(hit.body, query);
  const note = body.mark === null && hit.resolutionNote !== null ? snippetOf(hit.resolutionNote, query) : null;
  return { hit, body, note };
}

/** 項目の結果か（その日のボードの項目へ移る）。 */
export function isItemHit(hit: SearchHit): hit is SearchHit & { kind: ItemKind } {
  return hit.kind === "todo" || hit.kind === "blocker";
}

export class Palette {
  open = $state(false);
  /** 入力欄の文字。 */
  query = $state("");
  /** いま出している検索の答えと、その検索語（前後の空白を除いたもの）。 */
  views = $state.raw<HitView[]>([]);
  viewsFor = $state("");
  /** 検索を送るのを待っているか、送って答えを待っている（IME の変換中は送らないので、入らない）。 */
  searching = $state(false);
  /** いまの検索語で検索できなかった。 */
  failed = $state(false);
  /** 開く直前にフォーカスのあった所（閉じたら画面が戻す）。 */
  returnFocus: Element | null = null;

  #search: (text: string, limit: number) => Promise<SearchHit[]>;
  #delay: number;
  #timer: ReturnType<typeof setTimeout> | undefined;
  /** 送った検索の番号（後から送ったものの答えだけを使う）。 */
  #seq = 0;

  constructor(search: (text: string, limit: number) => Promise<SearchHit[]> = core.search, delay = SEARCH_DELAY_MS) {
    this.#search = search;
    this.#delay = delay;
  }

  /** 開く（入力は空から始める）。 */
  show() {
    if (this.open) return;
    this.returnFocus = document.activeElement;
    this.#reset();
    this.open = true;
  }

  /** 閉じる。待っている検索の答えは捨てる。 */
  close() {
    if (!this.open) return;
    this.open = false;
    this.#reset();
  }

  /** 入力が変わった。`composing` は IME の変換中か（変換中は検索を送らない）。 */
  input(query: string, composing = false) {
    this.query = query;
    if (composing) {
      // 変換を終えるまで待つ（変換の前の検索語で送らない）
      this.#cancel();
      return;
    }
    this.#schedule();
  }

  /** いまの入力の検索語（前後の空白を除いたもの。結果を出すかどうかもこれで決める）。 */
  get term(): string {
    return searchTerm(this.query);
  }

  /** 終わったら呼ぶ（待っている検索をやめる）。 */
  stop() {
    this.close();
  }

  #reset() {
    this.query = "";
    this.#clearResults();
  }

  /** 送るのを待っている検索と、答えを待っている検索をやめる（届いた答えは捨てる）。 */
  #cancel() {
    clearTimeout(this.#timer);
    this.#seq++;
    this.searching = false;
  }

  /** 出している結果と、検索の状態を消す。 */
  #clearResults() {
    this.#cancel();
    this.views = [];
    this.viewsFor = "";
    this.failed = false;
  }

  #schedule() {
    const term = this.term;
    if (term === "") {
      this.#clearResults();
      return;
    }
    if (term === this.viewsFor && !this.failed) {
      // 出している結果の検索語に戻った
      this.#cancel();
      return;
    }
    this.#cancel();
    // 前の検索語の失敗は、新しい検索語には当てはめない
    this.failed = false;
    this.searching = true;
    this.#timer = setTimeout(() => void this.#run(term), this.#delay);
  }

  async #run(term: string) {
    const n = ++this.#seq;
    try {
      const hits = await this.#search(term, SEARCH_LIMIT);
      if (n !== this.#seq) return;
      this.views = hits.map((h) => hitView(h, term));
      this.viewsFor = term;
      this.failed = false;
    } catch {
      if (n !== this.#seq) return;
      this.failed = true;
    } finally {
      if (n === this.#seq) this.searching = false;
    }
  }
}

// ---- 日付とコマンド ----

/** パレットから呼べるコマンド（画面設計書 6章）。 */
export type PaletteCommandId =
  | "carryover"
  | "saveDetail"
  | "today"
  | "mode"
  | "archive"
  | "agents"
  | "settings"
  | "sidebar"
  | "layoutA"
  | "layoutB"
  | "layoutC";

export interface PaletteCommand {
  id: PaletteCommandId;
  label: string;
  /** キーの表示（無ければ `null`）。 */
  key: string | null;
  /** いまの設定に当たる（「使用中」を出す）。 */
  active: boolean;
  /** 照合に使う、もう一方の言語の名前など。 */
  keywords: string;
}

/** コマンドの一覧を決める、いまの画面の状態。 */
export interface CommandContext {
  mode: Mode;
  sidebarOpen: boolean;
  layout: Layout;
  /** 今日の持ち越しが残っている（`Carryover.pending`）。 */
  carryPending: boolean;
  /** 保存に失敗している（原因の表示を開ける）。 */
  saveFailing: boolean;
  mac: boolean;
}

/**
 * コマンドの一覧（この順に出す）。持ち越しを開くは今日の持ち越しが残っている間だけ、保存できていない
 * 理由は保存に失敗している間だけ出す。`other` はもう一方の言語の文言（どちらの言語の名前でも探せる）。
 */
export function paletteCommands(ctx: CommandContext, m: Messages, other: Messages): PaletteCommand[] {
  const key = (a: ShellAction) => keyLabel(a, ctx.mac);
  const list: { id: PaletteCommandId; label: (x: Messages) => string; key?: string; active?: boolean; when?: boolean }[] = [
    { id: "carryover", label: (x) => x.palette.commands.carryover, when: ctx.carryPending },
    { id: "saveDetail", label: (x) => x.palette.commands.saveDetail, when: ctx.saveFailing },
    { id: "today", label: (x) => x.palette.commands.today, key: key("today") },
    {
      id: "mode",
      label: (x) => (ctx.mode === "board" ? x.palette.commands.toNote : x.palette.commands.toBoard),
      key: key("toggleMode"),
    },
    { id: "archive", label: (x) => x.palette.commands.archive, key: key("archive") },
    { id: "agents", label: (x) => x.palette.commands.agents },
    { id: "settings", label: (x) => x.palette.commands.settings, key: key("settings") },
    {
      id: "sidebar",
      label: (x) => (ctx.sidebarOpen ? x.palette.commands.closeSidebar : x.palette.commands.openSidebar),
      key: key("sidebar"),
    },
    { id: "layoutA", label: (x) => x.palette.commands.layout.A, active: ctx.layout === "A" },
    { id: "layoutB", label: (x) => x.palette.commands.layout.B, active: ctx.layout === "B" },
    { id: "layoutC", label: (x) => x.palette.commands.layout.C, active: ctx.layout === "C" },
  ];
  return list
    .filter((c) => c.when !== false)
    .map((c) => ({ id: c.id, label: c.label(m), key: c.key ?? null, active: c.active ?? false, keywords: c.label(other) }));
}

/**
 * 入力 `query` に当たるコマンド（名前のどこかに含む。英字の大文字・小文字と、空白・ハイフン・中点・
 * コロン・かっこの有無は問わない。「carry over」で「carry-over」に当たるように）。
 */
export function matchCommands(list: PaletteCommand[], query: string): PaletteCommand[] {
  const norm = (s: string) =>
    s
      .normalize("NFKC")
      .toLowerCase()
      .replace(/[\s\-_・:()]+/g, "");
  const q = norm(query);
  if (q === "") return list;
  return list.filter((c) => norm(c.label).includes(q) || norm(c.keywords).includes(q));
}

/** 入力が日付なら、移る先の日（`today` は今日の論理日付。まだ分からなければ日付として扱わない）。 */
export function dateTarget(query: string, today: LogicalDate | null): LogicalDate | null {
  return today === null ? null : parseDateQuery(query, today);
}
