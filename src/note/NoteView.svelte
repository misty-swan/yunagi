<script lang="ts">
  // ノートモードの本文（画面設計書 4章 S-02）：その日のノートのエディタ（CodeMirror 6）と、右の Outline・Sent。
  //
  // - 本文の幅は 680px で中央に置く。左に縦の細い線（時間軸）を引き、時刻付き見出しの位置に丸を置く。
  //   いまカーソルがある区間（その行より上で最も近い時刻付き見出しの区間）の丸は塗る。
  // - 時刻付き見出しは、時刻を大きく（26px・細字・夕陽色）、件名を 17px で続け、「## 」はごく薄く出す
  //   （デザインガイド 4章）。ほかの行（`#` の見出し・`**` などの強調も）は、書いたとおりの本文として、
  //   太さ 400 の直立で出す（2026-11-03 決定。デザインガイド 4章「ノートの等幅」の「太さ」）。
  // - Mod+Shift+H（ノートモードの間、エディタの外でも）で時刻付き見出しを入れる。「- 」の行の Enter で
  //   次の行も「- 」で始める（操作仕様書 2章「ノートの編集」）。
  // - Mod+Shift+T・B で、カーソルの行（選択範囲があればその行の全部）を TODO・障害へ送る。送った行には
  //   行末の外側に「→ TODO」「→ BLOCKER」を出し、右の Outline の下の Sent に行数を出す（src/lib/note_marks.ts）。
  //   整理モードの項目の「← NOTE」から移ってきたら（`jump`）、送り元の行にカーソルを置く。検索パレットの
  //   ノートの結果から移ってきたら、一致した所を選ぶ。
  // - 設定のノートの書体が等幅なら、同梱の等幅の書体を読み込み終えてから当てる（src/lib/fonts.svelte.ts）。
  //   起動時は、読み込み終えるまで（上限 FONT_WAIT_MS）ノートの文字を出さない。
  // - 保存・取り込み・入力中の状態・日の移動は、振り返りと共通（src/lib/doc_editor.ts）。
  import { isolateHistory } from "@codemirror/commands";
  import { RangeSetBuilder, StateField, type EditorState, type Transaction } from "@codemirror/state";
  import { Decoration, EditorView, type DecorationSet } from "@codemirror/view";
  import { onMount, tick, untrack } from "svelte";
  import type { SaveStatus } from "../lib/autosave.svelte";
  import type { ItemKind, LogicalDate, NoteSource } from "../lib/bindings";
  import { core } from "../lib/core";
  import { selectMatch, TextDocEditor } from "../lib/doc_editor";
  import type { NoteFind } from "../lib/palette.svelte";
  import { FONT_WAIT_MS, monoFont, type FontLoader } from "../lib/fonts.svelte";
  import { t } from "../lib/i18n";
  import { modShiftLabel, noteActionOf } from "../lib/keys";
  import { continueList, insertTimeHeading, timeHeadingOf, type Edit, type TimeHeading } from "../lib/note";
  import { markedLine, marksField, sendRange, sentCounts, setMarks } from "../lib/note_marks";
  import type { Notice } from "../lib/notice.svelte";
  import { prefs, type NoteFont } from "../lib/prefs.svelte";
  import type { EditorPresence } from "../lib/ui_report.svelte";

  let {
    date,
    save,
    presence,
    rev,
    onActivity,
    visible,
    narrow,
    mac,
    notice,
    items,
    itemsDate,
    jump,
    onJumped,
    font = monoFont,
    now = () => new Date(),
    view = $bindable(null),
  }: {
    /** 表示している日（ボードを読むまでは `null`）。 */
    date: LogicalDate | null;
    save: SaveStatus;
    presence: EditorPresence;
    /** ボードを読んだときのノートの版（進んだら、コアの最新を読み直して取り込む）。 */
    rev: number;
    onActivity: () => void;
    /** ノートモードか（キーを受け持つか）。 */
    visible: boolean;
    /** ウィンドウ幅が 900px 未満か（Outline を出さない）。 */
    narrow: boolean;
    mac: boolean;
    notice: Notice;
    /**
     * `itemsDate` の日のノートから送った項目（行送りで作った、その日の項目の id を `,` でつないだもの）。
     * 増えた・消えたら送信済みの印を求め直し、エディタの日の分が空なら、印は無いのでコアに問い合わせない。
     */
    items: string;
    /** `items` を読んだボードの日（日を移った直後は、エディタの日とずれる）。 */
    itemsDate: LogicalDate | null;
    /**
     * 移る先（無ければ `null`）：整理モードの項目の「← NOTE」からの送り元の行か、検索パレットの結果の
     * 一致した所（`find`）。移り終えたら `onJumped` を呼ぶ。
     */
    jump: NoteSource | NoteFind | null;
    onJumped: () => void;
    /** 等幅の書体の読み込み（テストで差し替える）。 */
    font?: FontLoader;
    /** いまの時刻（時刻付き見出しに入れる。テストで差し替える）。 */
    now?: () => Date;
    view?: EditorView | null;
  } = $props();

  const m = $derived(t());
  let host = $state<HTMLElement | null>(null);

  // ---- 時刻付き見出しと Outline ----

  interface Heading extends TimeHeading {
    /** 行の番号（1 始まり）。 */
    line: number;
  }

  /** 本文の時刻付き見出し（上から順）。 */
  function headingsOf(state: EditorState): Heading[] {
    const out: Heading[] = [];
    const doc = state.doc;
    for (let n = 1; n <= doc.lines; n++) {
      const line = doc.line(n);
      // 見出しは「## 」で始まる行だけなので、先に安く絞る
      if (!line.text.startsWith("## ")) continue;
      const h = timeHeadingOf(line.text);
      if (h !== null) out.push({ ...h, line: n });
    }
    return out;
  }

  /** カーソルのある区間の見出しの行（その行より上で最も近い見出し。無ければ 0）。 */
  function currentOf(state: EditorState, heads: readonly Heading[]): number {
    const at = state.doc.lineAt(state.selection.main.head).number;
    let cur = 0;
    for (const h of heads) {
      if (h.line > at) break;
      cur = h.line;
    }
    return cur;
  }

  const headLine = Decoration.line({ class: "cm-th" });
  const headLineCur = Decoration.line({ class: "cm-th cm-th-cur" });
  const mdMark = Decoration.mark({ class: "cm-md" });
  const timeMark = Decoration.mark({ class: "cm-tm" });

  interface Outline {
    heads: Heading[];
    cur: number;
    deco: DecorationSet;
  }

  const sameHeads = (a: readonly Heading[], b: readonly Heading[]) =>
    a.length === b.length &&
    a.every((h, i) => (Object.keys(h) as (keyof Heading)[]).every((k) => h[k] === b[i][k]));

  /**
   * 書き換え `tr` で見出しが変わりうるか。書き換えたどの所も、前と後で行の数が変わらず（ほかの行が
   * ずれない）、書き換えた行が前も後も「## 」で始まらなければ、見出しの一覧は前のまま（本文の行を打って
   * いる間に、本文全体を読み直さないため）。複数のカーソルで打つときも、所ごとに同じに確かめる。
   */
  function touchesHeadings(tr: Transaction): boolean {
    const before = tr.startState.doc;
    const after = tr.state.doc;
    if (before.lines !== after.lines) return true;
    const heading = (doc: typeof before, a: number, b: number) => {
      for (let n = a; n <= b; n++) {
        if (doc.line(n).text.startsWith("## ")) return true;
      }
      return false;
    };
    let touched = false;
    tr.changes.iterChangedRanges((fromA, toA, fromB, toB) => {
      if (touched) return;
      const [a1, a2] = [before.lineAt(fromA).number, before.lineAt(toA).number];
      const [b1, b2] = [after.lineAt(fromB).number, after.lineAt(toB).number];
      // 行の数が変わる所があれば、その間の見出しの行がずれる（全体では同じ数でも）
      touched = a2 - a1 !== b2 - b1 || heading(before, a1, a2) || heading(after, b1, b2);
    });
    return touched;
  }

  function outlineOf(state: EditorState, prev: Outline | null, tr: Transaction | null): Outline {
    let heads = prev?.heads ?? [];
    const touched = prev === null || (tr !== null && tr.docChanged && touchesHeadings(tr));
    if (touched) {
      const next = headingsOf(state);
      // 同じなら前の一覧を使う（Outline を描き直さないため）
      heads = prev !== null && sameHeads(next, prev.heads) ? prev.heads : next;
    }
    const cur = currentOf(state, heads);
    if (prev !== null && !tr?.docChanged && prev.cur === cur) return prev;
    // 見出しの行に触れず、区間も変わらなければ、飾りは書き換えに合わせて動かすだけ（1文字ごとに作り
    // 直さない）。見出しの行を書き換えたときは、同じ見出しになっても作り直す（書き換えた所の飾りは
    // 動かすと消えるため）
    if (prev !== null && tr !== null && !touched && cur === prev.cur) {
      return { heads, cur, deco: prev.deco.map(tr.changes) };
    }
    const b = new RangeSetBuilder<Decoration>();
    for (const h of heads) {
      const from = state.doc.line(h.line).from;
      b.add(from, from, h.line === cur ? headLineCur : headLine);
      b.add(from, from + h.timeFrom, mdMark);
      b.add(from + h.timeFrom, from + h.timeTo, timeMark);
    }
    return { heads, cur, deco: b.finish() };
  }

  /** 見出しの一覧と飾り。本文かカーソルが変わったときだけ求め直す。 */
  const outlineField = StateField.define<Outline>({
    create: (state) => outlineOf(state, null, null),
    update: (value, tr) => (tr.docChanged || tr.selection ? outlineOf(tr.state, value, tr) : value),
    provide: (f) => EditorView.decorations.from(f, (o) => o.deco),
  });

  let heads = $state.raw<Heading[]>([]);
  let cur = $state(0);
  let sent = $state<Record<ItemKind, number>>({ todo: 0, blocker: 0 });

  /** Outline と Sent を、エディタに合わせる（入力・カーソルの移動・本文と印の読み込みのたびに呼ぶ）。 */
  function syncOutline(state: EditorState) {
    const o = state.field(outlineField, false);
    if (o === undefined) return;
    if (o.heads !== heads) heads = o.heads;
    cur = o.cur;
    const marks = state.field(marksField, false);
    if (marks !== undefined) {
      const c = sentCounts(marks);
      if (c.todo !== sent.todo || c.blocker !== sent.blocker) sent = c;
    }
  }

  // ---- 編集 ----

  /** 書き換えを入れる。`isolate` なら、前後の入力と分けて1回の取り消しにする。 */
  function apply(v: EditorView, e: Edit, userEvent: string, isolate = false) {
    v.dispatch({
      changes: { from: e.from, to: e.to, insert: e.insert },
      selection: { anchor: e.caret },
      scrollIntoView: true,
      userEvent,
      annotations: isolate ? isolateHistory.of("full") : [],
    });
  }

  /** 時刻付き見出しを入れる（入力を受け付けていないときは何もしない）。 */
  function timeHeading(v: EditorView): boolean {
    if (!v.state.facet(EditorView.editable)) return false;
    const line = v.state.doc.lineAt(v.state.selection.main.head);
    // 見出しだけを1回の取り消しにする（続けて打つ件名とまとめない。操作仕様書 2章）
    apply(v, insertTimeHeading({ from: line.from, text: line.text }, now()), "input.timeHeading", true);
    return true;
  }

  function enter(v: EditorView): boolean {
    const sel = v.state.selection;
    if (v.composing || sel.ranges.length > 1 || !sel.main.empty) return false;
    const line = v.state.doc.lineAt(sel.main.head);
    const e = continueList({ from: line.from, text: line.text }, sel.main.head - line.from);
    if (e === null) return false;
    apply(v, e, "input");
    return true;
  }

  /** カーソルの行（選択範囲があればその行の全部）を `kind` へ送る（入力を受け付けていないときは何もしない）。 */
  async function sendLines(v: EditorView, kind: ItemKind) {
    const saver = presence.saver;
    if (!v.state.facet(EditorView.editable) || saver === null) return;
    // 送る前の入力と後の入力を、別の取り消しにする（操作仕様書 2章）
    v.dispatch({ annotations: isolateHistory.of("full") });
    try {
      // 送る行は、送る時点のエディタで決める（行き違いの取り込みで行がずれても）
      const r = await saver.sendLines(kind, () =>
        presence.saver === saver && view === v ? sendRange(v.state) : null,
      );
      if (r === null) return;
      if (r.result === "stale") {
        notice.show(t().sendFailed);
        return;
      }
      if (r.sent.length === 0) {
        const already = r.skipped.length > 0 && r.skipped.every((l) => l.reason === "already_sent");
        notice.show(already ? t().sendAlreadySent : t().sendNothing);
      } else if (presence.saver === saver) {
        sentHere = true;
      }
      refreshMarks(0);
    } catch {
      notice.show(t().sendFailed);
    }
  }

  // ---- 送信済みの印 ----

  /** 入力をやめてから印を求め直すまでの時間（打つたびにコアへ問い合わせないため）。 */
  const MARKS_DELAY_MS = 150;
  /** 印を求められなかったときに、求め直すまでの時間。 */
  const MARKS_RETRY_MS = 1000;

  /** 本文を読み終えた日（印を求める日）。 */
  let loaded = $state<LogicalDate | null>(null);
  /** 読み終えた本文で、印を一度求め終えたか（`jump` は、それを待ってから移る）。 */
  let marked = $state(false);
  let marksTimer: ReturnType<typeof setTimeout> | undefined;
  /** 印を求めた回数（後から始めた問い合わせの答えだけを使う）。 */
  let marksSeq = 0;
  /** 読み終えた後に、この画面から行を送ったか（ボードを読み直して `items` に入るまでも、印を求める）。 */
  let sentHere = false;

  /** `delay` の後に、いまのエディタの本文で印を求め直す（その間に打ったら、待ち直す）。 */
  function refreshMarks(delay = MARKS_DELAY_MS) {
    clearTimeout(marksTimer);
    marksTimer = setTimeout(() => void fetchMarks(), delay);
  }

  async function fetchMarks() {
    const v = view;
    const d = loaded;
    if (v === null || d === null) return;
    const n = ++marksSeq;
    const doc = v.state.doc;
    // その日のノートから送った項目が無ければ、印は無い（打つたびに本文をコアへ送らない）。ボードがまだ
    // ほかの日のものなら、分からないので問い合わせる
    if (itemsDate === d && items === "" && !sentHere) {
      if (v.state.field(marksField).marks.length > 0) v.dispatch({ effects: setMarks.of([]) });
      syncOutline(v.state);
      marked = true;
      return;
    }
    try {
      const marks = await core.noteMarks(d, doc.toString());
      // 求めている間に打った・日を移った・後から求め直したら、捨てる（打ったなら、その後にまた求める）
      if (n !== marksSeq || view !== v || loaded !== d || v.state.doc !== doc) return;
      v.dispatch({ effects: setMarks.of(marks) });
      syncOutline(v.state);
      marked = true;
    } catch {
      if (n !== marksSeq || view !== v || loaded !== d) return;
      // 求められなくても移る先は決める（保存済みの本文の行で移る）
      marked = true;
      refreshMarks(MARKS_RETRY_MS);
    }
  }

  // svelte-ignore state_referenced_locally
  const editor = new TextDocEditor({
    doc: "note",
    save,
    presence,
    // 入力・カーソルの移動・本文を読み終えたときに呼ばれる
    onActivity: () => {
      if (view !== null) syncOutline(view.state);
      onActivity();
    },
    extensions: () => ({
      extensions: [
        outlineField,
        marksField,
        EditorView.updateListener.of((u) => {
          if (u.docChanged) refreshMarks();
        }),
        EditorView.domEventHandlers({ contextmenu: onContextMenu }),
      ],
      keys: [{ key: "Enter", run: enter }],
    }),
    label: () => t().noteLabel,
    onLoaded: (d) => {
      loaded = d;
      marked = false;
      sentHere = false;
      refreshMarks(0);
    },
  });

  // その日のノートから送った項目が増えた・消えたら（送り先を消すと印が外れる）、印を求め直す
  $effect(() => {
    void items;
    const d = itemsDate;
    untrack(() => {
      // 送った後にボードを読み直したら、送った項目は `items` に入る（消した後も問い合わせ続けない）
      if (d === loaded) sentHere = false;
      refreshMarks(0);
    });
  });

  // 「← NOTE」から移る：その日の本文と印を読み終えてから、送り元の行の終わりにカーソルを置く。
  // エディタの本文（まだ保存していない入力を含む）の印で行を探し、無ければ保存済みの本文の行にする
  // 検索パレットの結果から移るときは、本文を読み終えたら（印は待たずに）一致した所を選ぶ
  $effect(() => {
    const j = jump;
    const isFind = j !== null && "find" in j;
    const ready = loaded !== null && loaded === j?.date && (isFind || marked);
    const v = view;
    if (j === null || !ready || v === null) return;
    untrack(() => {
      if ("find" in j) {
        selectMatch(v, j.find);
        onJumped();
        return;
      }
      const line = markedLine(v.state, j.itemId) ?? Math.min(j.line, v.state.doc.lines);
      const pos = v.state.doc.line(line).to;
      v.dispatch({ selection: { anchor: pos }, effects: EditorView.scrollIntoView(pos, { y: "center" }) });
      v.focus();
      onJumped();
    });
  });

  // 言語を変えたら、読み上げ用の名前を付け直す
  $effect(() => {
    void t();
    untrack(() => editor.relabel());
  });

  /** Outline の見出しへ移る（カーソルをその行の終わりに置き、エディタに戻る）。 */
  function toHeading(h: Heading) {
    const v = view;
    if (v === null) return;
    if (h.line > v.state.doc.lines) return;
    v.dispatch({ selection: { anchor: v.state.doc.line(h.line).to }, scrollIntoView: true });
    v.focus();
  }

  /**
   * ほかの入力欄（上部のバーの検索欄・パレット・設定など）にフォーカスがあるか。設定を開いている間は、
   * フォーカスがボタンにあるときも、ノートのキーを受けない（画面設計書 7章、操作仕様書 2章「ノートの編集」）。
   */
  function typingElsewhere(): boolean {
    const a = document.activeElement;
    if (!(a instanceof HTMLElement) || view?.dom.contains(a)) return false;
    return a.closest("[data-settings]") !== null || a.isContentEditable || a instanceof HTMLInputElement || a instanceof HTMLTextAreaElement || a instanceof HTMLSelectElement;
  }

  function onKeydown(e: KeyboardEvent) {
    if (!visible || e.defaultPrevented || view === null) return;
    const action = noteActionOf(e, mac);
    if (action === null || typingElsewhere()) return;
    e.preventDefault();
    // 押し続けたときの繰り返しでは、見出しを重ねて入れない・送り直さない
    if (e.repeat) return;
    if (!view.hasFocus) view.focus();
    if (action === "timeHeading") timeHeading(view);
    else void sendLines(view, action === "toTodo" ? "todo" : "blocker");
  }

  // ---- 行の右クリックのメニュー（操作仕様書 5章） ----

  /** 開いているメニューの位置（画面の座標）。閉じていれば `null`。 */
  let menu = $state<{ x: number; y: number } | null>(null);
  let menuEl = $state<HTMLElement | null>(null);

  /**
   * 右クリック：選択範囲の外を押したら、カーソルをその位置に移してから、送るメニューを開く（選択範囲の
   * 中なら、選択範囲の行を送る）。本文を読み終えるまでは開かない（ブラウザのメニューも出さない）。
   */
  function onContextMenu(e: MouseEvent, v: EditorView): boolean {
    e.preventDefault();
    if (!v.state.facet(EditorView.editable)) return true;
    const pos = v.posAtCoords({ x: e.clientX, y: e.clientY });
    const sel = v.state.selection.main;
    if (pos !== null && (pos < sel.from || pos > sel.to || sel.empty)) v.dispatch({ selection: { anchor: pos } });
    menu = { x: e.clientX, y: e.clientY };
    void tick().then(() => {
      if (menu === null || menuEl === null) return;
      // 窓の右・下の端の近くでは、窓の中に収まるように寄せる
      const r = menuEl.getBoundingClientRect();
      const x = Math.max(0, Math.min(menu.x, window.innerWidth - r.width - MENU_MARGIN_PX));
      const y = Math.max(0, Math.min(menu.y, window.innerHeight - r.height - MENU_MARGIN_PX));
      if (x !== menu.x || y !== menu.y) menu = { x, y };
      menuEl.querySelector<HTMLElement>("button")?.focus();
    });
    return true;
  }

  /** メニューを窓の端に寄せたときに、端から空ける間（px）。 */
  const MENU_MARGIN_PX = 4;

  /** メニューを閉じる。`back` ならエディタにフォーカスを戻す。 */
  function closeMenu(back: boolean) {
    if (menu === null) return;
    menu = null;
    if (back) view?.focus();
  }

  function fromMenu(kind: ItemKind) {
    const v = view;
    closeMenu(true);
    if (v !== null) void sendLines(v, kind);
  }

  function onMenuKey(e: KeyboardEvent) {
    const items = [...(menuEl?.querySelectorAll<HTMLElement>("button") ?? [])];
    const at = items.indexOf(document.activeElement as HTMLElement);
    if (e.key === "Escape") {
      e.preventDefault();
      closeMenu(true);
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const step = e.key === "ArrowDown" ? 1 : -1;
      items[(at + step + items.length) % items.length]?.focus();
    }
  }

  // メニューの外を押したら閉じる
  function onPointerDown(e: PointerEvent) {
    if (menu !== null && !(e.target instanceof Node && menuEl?.contains(e.target))) closeMenu(false);
  }

  // ノートモードを離れたら閉じる
  $effect(() => {
    if (!visible) untrack(() => closeMenu(false));
  });

  // ---- 書体 ----

  /** ノートに当てている書体（等幅は読み込み終えてから当てる）。 */
  let applied = $state<NoteFont>("body");
  /** ノートの文字を出すか（起動時に等幅を待つ間だけ偽）。 */
  let ready = $state(false);

  $effect(() => {
    const wanted = prefs.noteFont;
    // 設定が変わるか、部品を片付けたら、前の待ちと読み込みの結果は使わない
    let done = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    untrack(() => {
      if (wanted === "body") {
        setFont("body");
        ready = true;
        return;
      }
      if (font.status === "loaded") {
        setFont("mono");
        ready = true;
        return;
      }
      timer = setTimeout(() => (ready = true), FONT_WAIT_MS);
      void font.load().then((ok) => {
        if (done) return;
        clearTimeout(timer);
        if (ok) setFont("mono");
        ready = true;
      });
    });
    return () => {
      done = true;
      clearTimeout(timer);
    };
  });

  /** 書体を変えた後、出したときに測り直してカーソルの行を画面に残す（隠れている間は測れないため）。 */
  let remeasure = $state(false);

  /** 書体を変える。字の幅が変わり折り返しが変わるので、測り直して、カーソルの行を画面に残す。 */
  function setFont(next: NoteFont) {
    if (applied === next) return;
    applied = next;
    remeasure = true;
  }

  $effect(() => {
    if (!remeasure || !visible) return;
    untrack(() => {
      remeasure = false;
      void tick().then(() => {
        const v = view;
        if (v === null) return;
        v.requestMeasure();
        v.dispatch({ effects: EditorView.scrollIntoView(v.state.selection.main.head, { y: "nearest" }) });
      });
    });
  });

  onMount(() => {
    view = editor.mount(host!);
    syncOutline(view.state);
    return () => {
      clearTimeout(marksTimer);
      editor.destroy();
      view = null;
    };
  });

  $effect(() => {
    const d = date;
    if (view === null || d === null) return;
    untrack(() => {
      // 読み終えるまでは、読んだ日にしない（同じ日を読み直すときも。読んでいる途中の空の本文へ `jump` で
      // 移らないため）
      loaded = null;
      void editor.open(d);
    });
  });

  // ボードを読み込み直してノートの版が進んでいたら、最新を読み直して取り込む
  $effect(() => {
    const r = rev;
    const d = date;
    if (d !== null) untrack(() => editor.refresh(d, r));
  });
</script>

<svelte:window onkeydown={onKeydown} onpointerdown={onPointerDown} onblur={() => closeMenu(false)} />

<div class="note" data-font={applied} data-ready={ready ? "" : undefined} data-narrow={narrow ? "" : undefined}>
  <div class="wrap">
    <div class="note-ed" bind:this={host}></div>
    <aside class="note-side">
      <nav class="outline" aria-label={m.labels.outline}>
        <div class="label">{m.labels.outline}</div>
        {#each heads as h (h.line)}
          <button type="button" class:cur={h.line === cur} aria-current={h.line === cur ? "location" : undefined} onclick={() => toHeading(h)}>
            <span class="ot">{h.time}</span>
            {#if h.title !== ""}<span class="tt">{h.title}</span>{/if}
          </button>
        {/each}
      </nav>
      <section class="sent" aria-label={m.labels.sent}>
        <div class="label">{m.labels.sent}</div>
        <div class="counts">
          <span data-kind="todo">{m.labels.hint.toTodo} <b>{sent.todo}</b></span>
          <span data-kind="blocker">{m.labels.hint.toBlockers} <b>{sent.blocker}</b></span>
        </div>
      </section>
    </aside>
  </div>
</div>

{#if menu !== null}
  <div
    bind:this={menuEl}
    class="line-menu"
    role="menu"
    tabindex="-1"
    aria-label={m.lineMenu.label}
    style:left="{menu.x}px"
    style:top="{menu.y}px"
    onkeydown={onMenuKey}
  >
    <button type="button" role="menuitem" onclick={() => fromMenu("todo")}>
      {m.lineMenu.toTodo}<span class="key">{modShiftLabel("T", mac)}</span>
    </button>
    <button type="button" role="menuitem" onclick={() => fromMenu("blocker")}>
      {m.lineMenu.toBlocker}<span class="key">{modShiftLabel("B", mac)}</span>
    </button>
  </div>
{/if}

<style>
  .note {
    height: 100%;
    overflow: auto;
    scrollbar-width: thin;
    scrollbar-color: var(--rule) transparent;
  }

  .wrap {
    display: flex;
    justify-content: center;
    gap: calc(120 * var(--u));
    padding: calc(22 * var(--u)) var(--pad) 0;
    min-height: 100%;
  }

  .note-ed {
    width: calc(680 * var(--u));
    max-width: 100%;
    min-width: 0;
    display: flex;
    flex-direction: column;
    cursor: text;
  }

  /* 起動時に等幅を待つ間は、ノートの文字を出さない（読み込み終えてから描く。デザインガイド 4章） */
  .note:not([data-ready]) .note-ed {
    visibility: hidden;
  }

  .note-ed :global(.cm-editor) {
    flex: 1;
    background: transparent;
    color: var(--text);
    font: inherit;
  }

  .note-ed :global(.cm-editor.cm-focused) {
    outline: none;
  }

  .note-ed :global(.cm-scroller) {
    font: inherit;
    line-height: var(--lh-note);
    overflow: visible;
  }

  /* 左の時間軸：縦の細い線 */
  .note-ed :global(.cm-content) {
    min-height: 100%;
    padding: 0 0 40vh calc(44 * var(--u));
    caret-color: var(--accent-strong);
    background: linear-gradient(var(--rule), var(--rule)) no-repeat calc(7 * var(--u)) calc(12 * var(--u)) /
      1px calc(100% - 12 * var(--u));
  }

  .note[data-font="mono"] .note-ed :global(.cm-content) {
    font-family: var(--font-mono);
  }

  /* 時刻付き見出しを除き、どの行も太さ 400 の直立（等幅の書体は Regular だけを同梱するため。06 4章） */
  .note-ed :global(.cm-line) {
    padding: 0;
    font-weight: 400;
    font-style: normal;
  }

  .note-ed :global(.cm-cursor) {
    border-left-color: var(--accent-strong);
  }

  .note-ed :global(.cm-selectionBackground),
  .note-ed :global(.cm-focused .cm-selectionBackground) {
    background: var(--sel);
  }

  /* 時刻付き見出し：件名 17px・500、時刻 26px・300・夕陽色。等幅でも本文の書体のまま（06 4章） */
  /* 上の間は padding で空ける（CodeMirror は行の高さに margin を数えないため） */
  .note-ed :global(.cm-th) {
    position: relative;
    padding-top: calc(18 * var(--u));
    font-family: var(--font);
    font-size: calc(17 * var(--u));
    font-weight: 500;
    line-height: 1.6;
  }

  .note-ed :global(.cm-th:first-child) {
    padding-top: 0;
  }

  /* 時間軸の丸。カーソルのある区間の丸は塗る */
  .note-ed :global(.cm-th::before) {
    content: "";
    position: absolute;
    left: calc(-44 * var(--u));
    top: calc(32 * var(--u));
    width: calc(15 * var(--u));
    height: calc(15 * var(--u));
    border-radius: 50%;
    border: 1.5px solid var(--accent);
    background: var(--bg);
  }

  .note-ed :global(.cm-th:first-child::before) {
    top: calc(14 * var(--u));
  }

  .note-ed :global(.cm-th-cur::before) {
    background: var(--accent);
  }

  .note-ed :global(.cm-th .cm-md) {
    font-size: calc(11 * var(--u));
    font-weight: 400;
    color: var(--text-faint);
  }

  .note-ed :global(.cm-th .cm-tm) {
    margin-right: 4px;
    font-size: calc(26 * var(--u));
    font-weight: 300;
    color: var(--accent-strong);
    font-variant-numeric: tabular-nums;
  }

  /* 送信済みの行：行末の外側（本文の幅の右）に小さな英字で（デザインガイド 6章） */
  .note-ed :global(.cm-line[data-sent]) {
    position: relative;
  }

  .note-ed :global(.cm-line[data-sent]::after) {
    content: attr(data-sent);
    position: absolute;
    left: 100%;
    top: 0;
    margin-left: calc(16 * var(--u));
    white-space: pre;
    font-family: var(--font);
    font-size: calc(10.5 * var(--u));
    font-weight: 600;
    letter-spacing: 0.14em;
    color: var(--sea);
    line-height: calc(var(--fs-body) * var(--lh-note));
    pointer-events: none;
  }

  /* 制御文字（CR など）の印 */
  .note-ed :global(.cm-specialChar) {
    color: var(--text-faint);
  }

  /* 行の右クリックのメニュー（押した位置に重ねて出す） */
  .line-menu {
    position: fixed;
    z-index: 20;
    min-width: calc(180 * var(--u));
    padding: 4px;
    border: 1px solid var(--rule);
    border-radius: 6px;
    background: var(--surface);
    box-shadow: var(--shadow-pop);
  }

  .line-menu button {
    display: flex;
    justify-content: space-between;
    gap: 16px;
    width: 100%;
    padding: 6px 10px;
    border: 0;
    border-radius: 4px;
    background: transparent;
    text-align: left;
    cursor: pointer;
    font: inherit;
    font-size: var(--fs-small);
    color: var(--text);
  }

  .line-menu button:hover,
  .line-menu button:focus-visible {
    background: var(--hover);
    outline: none;
  }

  .line-menu .key {
    color: var(--text-muted);
  }

  .note-side {
    flex: none;
    width: calc(190 * var(--u));
    padding-top: calc(8 * var(--u));
    position: sticky;
    top: 0;
    align-self: flex-start;
  }

  .sent {
    margin-top: calc(26 * var(--u));
  }

  .sent .counts {
    display: flex;
    flex-direction: column;
    margin-top: 8px;
    font-size: var(--fs-small);
    color: var(--text-muted);
    line-height: 1.9;
  }

  .sent b {
    font-weight: 500;
    color: var(--text);
  }

  .outline button {
    display: flex;
    gap: 10px;
    width: 100%;
    margin-top: 8px;
    padding: 2px 0 2px 10px;
    border: 0;
    background: transparent;
    text-align: left;
    cursor: pointer;
    font-size: var(--fs-small);
    color: var(--text-muted);
    box-shadow: inset 2px 0 0 var(--rule);
  }

  .outline button:hover {
    color: var(--text);
  }

  .outline button.cur {
    color: var(--text);
    box-shadow: inset 2px 0 0 var(--accent);
  }

  .outline .ot {
    flex: none;
    font-variant-numeric: tabular-nums;
  }

  .outline .tt {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* 狭いウィンドウ（900px 未満）では Outline と Sent を出さず、本文を広く使う（画面設計書 4章）。
     送信済みの印は、本文の右の余白に収める */
  .note[data-narrow] .note-side {
    display: none;
  }

  .note[data-narrow] .note-ed :global(.cm-content) {
    padding-right: calc(96 * var(--u));
  }
</style>
