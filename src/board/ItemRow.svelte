<script lang="ts">
  // TODO・障害の1行（画面設計書 3章「各領域の表示」、デザインガイド 6章、操作仕様書 2章「一覧の編集」）。
  //
  // 行頭にチェック欄（TODO は丸、障害は角丸の四角）、本文（1行の入力欄。折り返す）、エージェントの名前、
  // 解消メモ（解消済みの障害の下の行に「→ メモ」）、行末に持ち越し先（「→ 11.03」）・取りやめた理由・
  // 持ち越し回数。持ち越し済みの項目と取りやめた TODO は、本文を入力できない（入力しようとしたら知らせる）。
  import CarryIcon from "../shell/CarryIcon.svelte";
  import type { CarriedTo, NoteSource } from "../lib/bindings";
  import { lockOf, showsMemo, type BoardModel, type FocusTarget, type Row } from "../lib/board.svelte";
  import { boardActionOf } from "../lib/board_keys";
  import { asVimKeyEvent, composing, vimKeyOfCode } from "../lib/keys";
  import { monthDay, shortDate } from "../lib/dates";
  import { agentLabel, t } from "../lib/i18n";
  import { prefs } from "../lib/prefs.svelte";
  import { caretCollapsed, caretOffset, caretOnEdgeLine, setCaret } from "./caret";

  let {
    row,
    model,
    mac,
    reviewAt,
    focusReq,
    onFocus,
    onNeighbor,
    onRemoving,
    onEdge,
    onRegion,
    onOpenCarried,
    onOpenNoteSource,
    onActivity,
  }: {
    row: Row;
    model: BoardModel;
    mac: boolean;
    /** 持ち越し回数をこの値以上で `--warn` にする。 */
    reviewAt: number;
    /** フォーカスを置く先（変わるたびに、自分の行なら置く）。 */
    focusReq: (FocusTarget & { n: number }) | null;
    /** 操作の後のフォーカスの行き先を、一覧に頼む。 */
    onFocus: (target: FocusTarget | null) => void;
    /** 隣の行へ移る（↑ / ↓）。`offset` はいまのカーソルの位置。 */
    onNeighbor: (key: string, dir: -1 | 1, offset: number) => void;
    /** この行を消す前に、フォーカスを隣の行（`dir` の向き。無ければ反対の隣）へ移す。 */
    onRemoving: (key: string, dir: -1 | 1) => void;
    /** 領域の先頭（-1）・末尾（+1）の行へ移る（Vim の gg・G）。 */
    onEdge: (dir: -1 | 1) => void;
    /** 前（-1）・次（+1）の領域へ移る（Vim の `[[`・`]]`、Shift+Tab・Tab）。 */
    onRegion: (dir: -1 | 1) => void;
    onOpenCarried: (to: CarriedTo) => void;
    /** 「← NOTE」：送り元のノートの行へ移る。 */
    onOpenNoteSource: (source: NoteSource) => void;
    /** 入力・フォーカスが変わった（入力中の状態を知らせ直す）。 */
    onActivity: () => void;
  } = $props();

  const m = $derived(t());
  const lock = $derived(lockOf(row));
  const closed = $derived(row.status === "done" || row.status === "resolved");
  const memoEditing = $derived(model.memoOpen === row.key);
  /** Vim のノーマルモードか（行を選ぶだけで、本文は入力欄にしない。操作仕様書 4.3）。 */
  const vimNormal = $derived(prefs.vimMode && !model.insert);
  /** 本文を入力欄にするか（変えられない行は、入力欄にしない）。 */
  const editable = $derived(lock === null && !vimNormal);

  let li = $state<HTMLElement | null>(null);
  let textEl = $state<HTMLElement | null>(null);
  let memoEl = $state<HTMLElement | null>(null);
  /** 変換を確定した直後の Enter を無視する（WebKit 系。操作仕様書 3章）。 */
  let justComposed = false;

  // 本文を、入力欄の中身と違うときだけ書き換える（エージェントの変更。カーソルは同じ文字位置に保つ）
  $effect(() => {
    syncText(textEl, row.text);
  });
  $effect(() => {
    syncText(memoEl, row.resolutionNote ?? "");
  });

  function syncText(el: HTMLElement | null, text: string) {
    if (el === null || el.textContent === text) return;
    const focused = el.ownerDocument.activeElement === el;
    const offset = focused ? caretOffset(el) : null;
    el.textContent = text;
    if (focused) setCaret(el, offset ?? "end");
  }

  // 頼まれたらフォーカスを置く
  $effect(() => {
    const req = focusReq;
    if (req === null || req.key !== row.key) return;
    // 入力欄ができるまで（挿入モードに入った直後）は待つ。できたら（textEl が変わったら）置く
    const el = req.field === "memo" ? memoEl : editable ? textEl : li;
    if (el === null) return;
    el.focus({ preventScroll: true });
    if (el !== li) setCaret(el, req.offset);
    el.scrollIntoView?.({ block: "nearest" });
  });

  // ---- Vim（操作仕様書 4.3） ----

  /** 挿入モードを抜けるキー（Esc と Ctrl+[。操作仕様書 4.4。macOS の Ctrl は Mod でないので、どの OS でも受ける）。 */
  function isVimEsc(e: KeyboardEvent): boolean {
    if (composing(e)) return false;
    if (e.altKey || e.metaKey || e.shiftKey) return false;
    return (e.key === "Escape" && !e.ctrlKey) || (e.key === "[" && e.ctrlKey);
  }

  /** ノーマルモードへ戻る。行そのものへフォーカスを移してから、入力欄を外す。 */
  function leaveInsert() {
    li?.focus({ preventScroll: true });
    model.insert = false;
    model.vimPending = null;
  }

  /** 挿入モードに入って、この行の本文にカーソルを置く（`i` は行頭、`a` は行末）。 */
  function enterInsert(at: number | "end") {
    model.insert = true;
    onFocus({ key: row.key, field: "text", offset: at });
  }

  /** 完了・解消を切り替える。障害を解消して解消メモの欄が開くときは、Vim では挿入モードとして開く（入力欄のため）。 */
  function toggleRow() {
    const target = model.toggle(row.key);
    if (target?.field === "memo" && prefs.vimMode) model.insert = true;
    onFocus(target);
  }

  /** ダブルクリックで編集する（Vim モードの項目のクリックは選ぶだけ。操作仕様書 5章）。 */
  function onDblClick(e: MouseEvent) {
    if (!vimNormal || (e.target as Element).closest(".chk, .memo, .carried-to, .from-note") !== null) return;
    if (model.refuseEdit(row.key)) return;
    enterInsert("end");
  }

  /**
   * ノーマルモードのキー。操作に当たれば `true`。続きのキーを待つ `g`・`d` は `model.vimPending` に持つ
   * （続きでないキーが来たら捨てる）。Mod・Alt との組み合わせはここで扱わない（標準のショートカット）。
   */
  function onVimKey(e: KeyboardEvent, pending: "g" | "d" | "]" | "[" | null): boolean {
    if (e.ctrlKey || e.metaKey || e.altKey) return false;
    switch (e.key) {
      case "j":
      case "k":
        onNeighbor(row.key, e.key === "j" ? 1 : -1, 0);
        return true;
      case "g":
        if (pending === "g") onEdge(-1);
        else model.vimPending = "g";
        return true;
      case "G":
        onEdge(1);
        return true;
      case "d":
        if (pending === "d") {
          onRemoving(row.key, 1);
          model.remove(row.key);
        } else model.vimPending = "d";
        return true;
      case "o":
      case "O": {
        // 足すだけで本文を変えないので、変えられない行にも使える
        const target = model.add(row.kind, row.key, e.key === "o" ? "after" : "before");
        if (target !== null) {
          model.insert = true;
          onFocus(target);
        }
        return true;
      }
      case "i":
      case "a":
        // 本文を変える操作：変えられない行では、本文・状態を変えずに知らせる
        if (!model.refuseEdit(row.key)) enterInsert(e.key === "i" ? 0 : "end");
        return true;
      case "x":
        if (model.refuseEdit(row.key)) return true;
        toggleRow();
        return true;
      case "J":
      case "K":
        model.move(row.key, e.key === "J" ? 1 : -1);
        onFocus({ key: row.key, field: "text", offset: 0 });
        return true;
      case "u":
        void model.undo();
        return true;
      case "Tab":
        onRegion(e.shiftKey ? -1 : 1);
        return true;
      case "]":
      case "[":
        // Vim の「次・前のセクションへ」。2打目で領域を移る（1打目は続きのキー待ち）
        if (pending === e.key) onRegion(e.key === "]" ? 1 : -1);
        else model.vimPending = e.key;
        return true;
    }
    return false;
  }

  /** 1行にする（改行は空白にする）。 */
  const oneLine = (s: string) => s.replace(/\s*[\r\n]+\s*/g, " ");

  function onTextInput() {
    if (textEl === null) return;
    const v = textEl.textContent ?? "";
    const one = oneLine(v);
    if (one !== v) {
      textEl.textContent = one;
      setCaret(textEl, "end");
    }
    model.editText(row.key, one);
    onActivity();
  }

  function onMemoInput() {
    if (memoEl === null) return;
    model.editMemo(row.key, oneLine(memoEl.textContent ?? ""));
    onActivity();
  }

  function onBeforeInput(e: InputEvent) {
    const type = e.inputType;
    if (type === "insertParagraph" || type === "insertLineBreak" || type.startsWith("format")) {
      e.preventDefault();
    } else if (type === "historyUndo" || type === "historyRedo") {
      e.preventDefault();
      void (type === "historyUndo" ? model.undo() : model.redo());
    }
  }

  // 貼り付けは文字だけを、改行を空白にして入れる
  function onPaste(e: ClipboardEvent) {
    e.preventDefault();
    const text = oneLine(e.clipboardData?.getData("text/plain") ?? "");
    const doc = (e.currentTarget as HTMLElement).ownerDocument;
    if (!doc.execCommand?.("insertText", false, text)) {
      const sel = doc.getSelection();
      if (sel === null || sel.rangeCount === 0) return;
      const r = sel.getRangeAt(0);
      r.deleteContents();
      r.insertNode(doc.createTextNode(text));
      r.collapse(false);
      (e.currentTarget as HTMLElement).dispatchEvent(new Event("input", { bubbles: true }));
    }
  }

  function onCompositionEnd() {
    model.setComposing(false);
    justComposed = true;
    setTimeout(() => (justComposed = false), 0);
    onActivity();
  }

  function onTextKey(e: KeyboardEvent) {
    // IME の変換中のキー（候補を選ぶ ↑↓ など）は、IME に任せる（操作仕様書 3章）
    if (e.defaultPrevented) return;
    // 変えられない行と、Vim のノーマルモードでは、行そのもの（li）へのキーだけを扱う（行の中のボタンの
    // キーは、そのボタンに任せる）
    if ((lock !== null || vimNormal) && e.target !== li) return;
    // Vim のノーマルモードは、IME の状態にかかわらずキーを Vim の操作として扱う（操作仕様書 4.4 の 2）。
    // IME が先に受けたキー（変換中の印が付く）は、押した物理キーから読み直す（読めなければ何もしない。
    // 本文は入力欄でないので、変換は始まらない）
    if (vimNormal && composing(e)) {
      const key = vimKeyOfCode(e);
      if (key === null) return;
      e.preventDefault();
      e = asVimKeyEvent(e, key);
    }
    if (composing(e)) {
      // 変えられない行では、日本語入力の変換を始めようとしたことを知らせる（本文は入力欄でないので、通常は届かない）
      if (lock !== null) model.refuseEdit(row.key);
      return;
    }
    // Vim：続きのキー待ち（g・d）は、次のキー（修飾キーだけの押下は除く）で必ず捨てる。ほかの操作を挟んだら続きにならない
    const pending = model.vimPending;
    if (!["Shift", "Control", "Alt", "Meta", "AltGraph"].includes(e.key)) model.vimPending = null;
    // 貼り付け・切り取りのキー（クリップボードのイベントが届かない WebView でも知らせる）
    if (lock !== null && (mac ? e.metaKey : e.ctrlKey) && !e.altKey && (e.code === "KeyV" || e.code === "KeyX")) {
      e.preventDefault();
      model.refuseEdit(row.key);
      return;
    }
    if (justComposed && e.key === "Enter") {
      justComposed = false;
      e.preventDefault();
      return;
    }
    const action = boardActionOf(e, mac);
    const el = textEl;
    const offset = el === null ? 0 : (caretOffset(el) ?? row.text.length);
    const plain = !e.altKey && !e.shiftKey && !(mac ? e.metaKey || e.ctrlKey : e.ctrlKey || e.metaKey);
    // Vim：挿入モードの Esc（非 macOS は Ctrl+[ も）でノーマルモードへ戻る
    if (prefs.vimMode && model.insert && isVimEsc(e)) {
      e.preventDefault();
      leaveInsert();
      return;
    }
    switch (action) {
      case "enter":
        e.preventDefault();
        // ノーマルモードの Enter は何もしない（行を分けると本文を変える。足すのは o / O）
        if (vimNormal) return;
        onFocus(model.splitAt(row.key, lock === null ? offset : row.text.length));
        return;
      case "toggle":
        e.preventDefault();
        toggleRow();
        return;
      case "delete":
        e.preventDefault();
        onRemoving(row.key, 1);
        model.remove(row.key);
        return;
      case "moveUp":
      case "moveDown":
        e.preventDefault();
        model.move(row.key, action === "moveUp" ? -1 : 1);
        onFocus({ key: row.key, field: "text", offset });
        return;
      case "handoff":
        e.preventDefault();
        // 押し続けたときの繰り返しでは書き出さない（Mod+Shift+T・B と同じ）
        if (!e.repeat) void model.handoff(row.key);
        return;
      case "undo":
      case "redo":
        e.preventDefault();
        void (action === "undo" ? model.undo() : model.redo());
        return;
    }
    // Vim のノーマルモード：文字のキーは Vim の操作（本文は入力欄でないので、文字は入らない）
    if (vimNormal) {
      if (onVimKey(e, pending)) {
        e.preventDefault();
        return;
      }
      // 操作に当たらない文字・Backspace・Delete は、何も起こさない（変えられない行でも知らせない）
      if (isTyping(e)) {
        e.preventDefault();
        return;
      }
    }
    // 変えられない行で文字を入力しようとした（Shift 付きの大文字、Option・AltGr で打つ文字も含む）
    if (lock !== null && isTyping(e)) {
      e.preventDefault();
      model.refuseEdit(row.key);
      return;
    }
    if (!plain) return;
    if (e.key === "Backspace" && el !== null && row.text === "" && caretCollapsed(el)) {
      e.preventDefault();
      onRemoving(row.key, -1);
      model.remove(row.key);
    } else if (e.key === "ArrowUp" && (el === null || caretOnEdgeLine(el, "first"))) {
      e.preventDefault();
      onNeighbor(row.key, -1, offset);
    } else if (e.key === "ArrowDown" && (el === null || caretOnEdgeLine(el, "last"))) {
      e.preventDefault();
      onNeighbor(row.key, 1, offset);
    }
  }

  /** 文字を打つキーか（Mod・Ctrl との組み合わせは除く。AltGr は Ctrl+Alt として届くので文字を打つ）。 */
  function isTyping(e: KeyboardEvent): boolean {
    // Mod との組み合わせ（Mod+Backspace の削除など）は、呼ぶ前に操作として扱い終えている
    if (e.key === "Backspace" || e.key === "Delete") return true;
    if (e.key.length !== 1) return false;
    if (e.getModifierState?.("AltGraph") === true) return true;
    // Windows の AltGr は Ctrl+Alt として届く（AltGraph を報告しない WebView もある）
    if (!mac && e.ctrlKey && e.altKey && !e.metaKey) return true;
    if (e.ctrlKey || e.metaKey) return false;
    // Windows・Linux の Alt+文字はメニューなどのキー。文字を打つのは macOS の Option だけ
    return mac || !e.altKey;
  }

  // 変えられない行への貼り付け・切り取り・文のドロップ（入力欄が無いので何も起きないが、知らせる）
  function onLockedEdit(e: Event) {
    // 行の中の入力欄（行が変えられなくなったとき開いていた解消メモ）の貼り付けは、その欄に任せる
    if ((e.target as Element | null)?.closest?.("[contenteditable]")) return;
    e.preventDefault();
    model.refuseEdit(row.key);
  }

  /** 文や URL・ファイルを運ぶ drag か（行の並べ替えは別の仕組みで、ここで受けない）。 */
  const carriesContent = (e: DragEvent) =>
    [...(e.dataTransfer?.types ?? [])].some((t) => t === "Files" || t === "text/plain" || t === "text/html" || t === "text/uri-list");

  function onLockedDragOver(e: DragEvent) {
    // drop が届くよう、運んでいるものがあれば受ける
    if (carriesContent(e)) e.preventDefault();
  }

  function onLockedDrop(e: DragEvent) {
    if (carriesContent(e)) onLockedEdit(e);
  }

  function onMemoKey(e: KeyboardEvent) {
    if (e.defaultPrevented || composing(e)) return;
    if (justComposed && e.key === "Enter") {
      justComposed = false;
      e.preventDefault();
      return;
    }
    const action = boardActionOf(e, mac);
    if (action === "toggle") {
      e.preventDefault();
      onFocus(model.toggle(row.key) ?? { key: row.key, field: "text", offset: "end" });
      return;
    }
    if (action === "enter" || e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      model.closeMemo();
      // Vim では、解消メモを書き終えたらノーマルモードに戻る
      if (prefs.vimMode) model.insert = false;
      onFocus({ key: row.key, field: "text", offset: "end" });
    }
  }

  function onCheck(e: MouseEvent) {
    e.preventDefault();
    onFocus(model.toggle(row.key) ?? { key: row.key, field: "text", offset: "end" });
  }

  function onFocusIn(e: FocusEvent) {
    // 続きのキーを待っていた g・d は、別の行へ移ったら捨てる（続けて押したことにならないため）
    if (model.focus?.key !== row.key) model.vimPending = null;
    model.setFocus({ key: row.key, field: e.target === memoEl ? "memo" : "text" });
    onActivity();
  }

  // 行の外へフォーカスが移ったら、後始末をする（まだ保存していない入力を送る・空の行を消す）
  function onFocusOut(e: FocusEvent) {
    const next = e.relatedTarget;
    if (next instanceof Node && li?.contains(next)) {
      if (e.target === memoEl && next !== memoEl) model.closeMemo();
      return;
    }
    if (model.focus?.key === row.key) model.setFocus(null);
    model.leave(row.key);
    onActivity();
  }
</script>

<!-- 変えられない行は、本文の入力欄の代わりに行そのものでキーを受ける（入力しようとしたら知らせる） -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<li
  bind:this={li}
  class="item"
  class:done={closed || row.status === "dropped"}
  class:dropped={row.status === "dropped"}
  class:locked={lock !== null}
  data-key={row.key}
  data-status={row.status}
  tabindex="-1"
  onfocusin={onFocusIn}
  onfocusout={onFocusOut}
  onkeydown={lock !== null || vimNormal ? onTextKey : undefined}
  ondblclick={onDblClick}
  onpaste={lock !== null ? onLockedEdit : undefined}
  oncut={lock !== null ? onLockedEdit : undefined}
  ondragover={lock !== null ? onLockedDragOver : undefined}
  ondrop={lock !== null ? onLockedDrop : undefined}
>
  {#if row.unseen}
    <span class="mk-agent lead" role="img" aria-label={m.agentUnseen} title={m.agentUnseen}></span>
  {/if}
  <button
    type="button"
    class="chk"
    class:box={row.kind === "blocker"}
    tabindex="-1"
    aria-label={row.kind === "todo" ? m.checkTodo : m.checkBlocker}
    aria-pressed={closed}
    onmousedown={(e) => e.preventDefault()}
    onclick={onCheck}
  >
    <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3.5 8.5 6.5 11.5 12.5 4.5" /></svg>
  </button>
  <span class="body">
    {#if editable}
      <span
        bind:this={textEl}
        class="txt"
        role="textbox"
        tabindex="0"
        aria-multiline="false"
        contenteditable="true"
        spellcheck="false"
        oninput={onTextInput}
        onbeforeinput={onBeforeInput}
        onpaste={onPaste}
        onkeydown={onTextKey}
        oncompositionstart={() => model.setComposing(true)}
        oncompositionend={onCompositionEnd}
      ></span>
    {:else}
      <span class="txt">{row.text}</span>
    {/if}
    {#if row.origin === "agent" && row.agentName !== null}
      <span class="agent-tag" title={m.agentItem}><span class="mk-agent" aria-hidden="true"></span>{agentLabel(row.agentName)}</span>
    {/if}
    {#if memoEditing}
      <span
        bind:this={memoEl}
        class="memo-edit"
        role="textbox"
        tabindex="0"
        aria-label={m.memoPlaceholder}
        data-ph={m.memoPlaceholder}
        contenteditable="true"
        spellcheck="false"
        oninput={onMemoInput}
        onbeforeinput={onBeforeInput}
        onpaste={onPaste}
        onkeydown={onMemoKey}
        oncompositionstart={() => model.setComposing(true)}
        oncompositionend={onCompositionEnd}
      ></span>
    {:else if showsMemo(row)}
      <button
        type="button"
        class="memo"
        tabindex="-1"
        onmousedown={(e) => e.preventDefault()}
        onclick={() => model.openMemo(row.key) && onFocus({ key: row.key, field: "memo", offset: "end" })}
      >
        → {row.resolutionNote}
      </button>
    {/if}
  </span>
  {#if row.noteSource !== null}
    {@const source = row.noteSource}
    <button
      type="button"
      class="from-note"
      tabindex="-1"
      aria-label={m.openNoteSource}
      title={m.openNoteSource}
      onmousedown={(e) => e.preventDefault()}
      onclick={() => onOpenNoteSource(source)}
    >
      {m.labels.fromNote}
    </button>
  {/if}
  {#if row.status === "dropped" && row.closeReason !== null}
    <span class="cstate">{m.closeReason[row.closeReason]}</span>
  {/if}
  {#if row.carriedTo !== null}
    {@const to = row.carriedTo}
    <button
      type="button"
      class="carried-to"
      tabindex="-1"
      aria-label={m.openCarriedTo(shortDate(to.date))}
      title={m.openCarriedTo(shortDate(to.date))}
      onmousedown={(e) => e.preventDefault()}
      onclick={() => onOpenCarried(to)}
    >
      → {monthDay(to.date)}
    </button>
  {/if}
  {#if row.carryCount > 0}
    <span
      class="carry"
      class:warn={row.carryCount >= reviewAt}
      role="img"
      aria-label={m.carryTimes(row.carryCount)}
      title={m.carryTimes(row.carryCount)}
    >
      <CarryIcon />{row.carryCount}
    </span>
  {/if}
</li>

<style>
  .item {
    position: relative;
    display: flex;
    align-items: flex-start;
    gap: calc(14 * var(--u));
    min-height: var(--row-h);
    margin-left: calc(-14 * var(--u));
    padding: calc((var(--row-h) - var(--fs-body) * var(--lh-body)) / 2) calc(12 * var(--u))
      calc((var(--row-h) - var(--fs-body) * var(--lh-body)) / 2) calc(14 * var(--u));
    border-radius: 0 var(--radius) var(--radius) 0;
    cursor: text;
  }

  /* 選択中の項目：左端に 2px の線と、薄い夕陽色の面（06 6章） */
  .item:focus,
  .item:focus-within {
    box-shadow: inset 2px 0 0 var(--accent-strong);
    background: var(--sel);
  }

  .item.locked {
    cursor: default;
  }

  .mk-agent {
    display: inline-block;
    flex: none;
    width: 6px;
    height: 6px;
    background: var(--agent);
    transform: rotate(45deg);
  }

  .lead {
    position: absolute;
    left: calc(-2 * var(--u));
    top: calc(var(--row-h) / 2 - 3px);
  }

  .chk {
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: calc(16 * var(--u));
    height: calc(16 * var(--u));
    margin-top: calc((var(--fs-body) * var(--lh-body) - 16 * var(--u)) / 2);
    padding: 0;
    border: 1.3px solid var(--ring);
    border-radius: 50%;
    background: transparent;
    cursor: pointer;
  }

  .chk.box {
    border-radius: calc(4 * var(--u));
  }

  .chk svg {
    width: 62%;
    height: 62%;
    fill: none;
    stroke: var(--bg);
    stroke-width: 2.6;
    stroke-linecap: round;
    stroke-linejoin: round;
    visibility: hidden;
  }

  .item.done:not(.dropped) .chk {
    background: var(--accent);
    border-color: var(--accent);
  }

  .item.done:not(.dropped) .chk svg {
    visibility: visible;
  }

  .item.dropped .chk {
    border-style: dashed;
    cursor: default;
  }

  .body {
    flex: 1 1 auto;
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .txt {
    outline: none;
    white-space: pre-wrap;
  }

  .txt:empty {
    display: inline-block;
    min-width: 2px;
  }

  .item.done .txt {
    color: var(--done);
    text-decoration: line-through;
    text-decoration-color: var(--done);
    text-decoration-thickness: 1px;
  }

  .agent-tag {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-left: 12px;
    font-size: calc(11.5 * var(--u));
    color: var(--agent);
    white-space: nowrap;
    vertical-align: 1px;
  }

  .agent-tag .mk-agent {
    width: 5px;
    height: 5px;
  }

  .memo {
    display: block;
    padding: 0;
    border: 0;
    background: none;
    text-align: left;
    font-size: var(--fs-small);
    color: var(--text-muted);
    cursor: text;
  }

  .memo-edit {
    display: block;
    font-size: var(--fs-small);
    outline: none;
    color: var(--text);
    white-space: pre-wrap;
  }

  .memo-edit:empty::before {
    content: attr(data-ph);
    color: var(--text-muted);
  }

  .carry,
  .cstate,
  .carried-to {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: var(--fs-meta);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
    line-height: calc(var(--fs-body) * var(--lh-body));
  }

  .carry.warn {
    color: var(--warn);
    font-weight: 500;
  }

  .carried-to {
    padding: 0;
    border: 0;
    background: none;
    cursor: pointer;
  }

  .carried-to:hover {
    color: var(--text);
  }

  /* 送り元のノートの行への印（ノートの「→ TODO」と対の、--sea の小さな英字。デザインガイド 6章） */
  .from-note {
    flex: none;
    padding: 0;
    border: 0;
    background: none;
    cursor: pointer;
    font-family: var(--font);
    font-size: calc(10.5 * var(--u));
    font-weight: 600;
    letter-spacing: 0.14em;
    color: var(--sea);
    line-height: calc(var(--fs-body) * var(--lh-body));
    white-space: nowrap;
  }

  .from-note:hover {
    text-decoration: underline;
  }
</style>
