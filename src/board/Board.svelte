<script lang="ts">
  // 整理モードの本文（画面設計書 3章「レイアウト」「各領域の表示」「空の状態」）：TODO・障害・振り返りの
  // 3領域を、設定のレイアウト（A. 2段・B. 3列・C. 縦積み）で並べる。ウィンドウ幅が 900px 未満なら、
  // 設定にかかわらず C にする（設定は変えない）。
  //
  // 受け持つキー（操作仕様書 2章「整理」）：Mod+1 / 2 / 3 で領域を移る。一覧の中のキーは ItemRow、
  // 振り返りのキーは ReflectionEditor。入力中の状態は、画面全体の `InputTracker`（App が持つ）に入れ、
  // 変わるたびに `report_ui_state` で知らせる（docs/design/01-architecture.md 4.1）。
  import type { EditorView } from "@codemirror/view";
  import { untrack } from "svelte";
  import { ItemSaves, type ItemSaveEvents, type SaveStatus } from "../lib/autosave.svelte";
  import type { CarriedTo, Id, ItemKind, LogicalDate, NoteSource } from "../lib/bindings";
  import { BoardModel, type BoardNotice, type FocusTarget } from "../lib/board.svelte";
  import { boardActionOf } from "../lib/board_keys";
  import { shortDate } from "../lib/dates";
  import { t } from "../lib/i18n";
  import { imeEnterInsert, imeLeaveInsert } from "../lib/ime.svelte";
  import type { Notice } from "../lib/notice.svelte";
  import { prefs, type Layout } from "../lib/prefs.svelte";
  import type { Session } from "../lib/session.svelte";
  import type { BoardTarget } from "../lib/palette.svelte";
  import type { InputTracker } from "../lib/ui_report.svelte";
  import ItemList from "./ItemList.svelte";
  import ReflectionEditor from "./ReflectionEditor.svelte";

  let {
    session,
    save,
    notice,
    narrow,
    mac,
    tracker,
    onOpenNoteSource,
    target = null,
    onTargetDone = () => {},
  }: {
    session: Session;
    save: SaveStatus;
    notice: Notice;
    /** 画面全体の入力中の状態。 */
    tracker: InputTracker;
    /** ウィンドウ幅が 900px 未満か。 */
    narrow: boolean;
    mac: boolean;
    /** 項目の「← NOTE」：送り元のノートの行へ移る（ノートモードへ移すのは App）。 */
    onOpenNoteSource: (source: NoteSource) => void;
    /** 検索パレットの結果から移る先（項目か、振り返りの中の一致した所）。移り終えたら `onTargetDone`。 */
    target?: BoardTarget | null;
    onTargetDone?: () => void;
  } = $props();

  const m = $derived(t());

  function noticeText(n: BoardNotice): string {
    const msg = t();
    switch (n.kind) {
      case "carried":
        return msg.carriedNotice(shortDate(n.date));
      case "dropped":
        return msg.droppedNotice;
      case "restore_source_changed":
        return msg.restoreSourceChanged;
      case "restore_source_carried":
        return msg.restoreSourceCarried(shortDate(n.date));
      case "rejected":
        return msg.saveRejected;
      case "handoff_copied":
        return msg.handoffCopied;
      case "handoff_not_todo":
        return msg.handoffNotTodo;
      case "handoff_failed":
        return msg.handoffFailed;
    }
  }

  // 保存の状態（save）は、アプリの間ずっと同じものを渡される
  const events: ItemSaveEvents = {};
  // svelte-ignore state_referenced_locally
  const items = new ItemSaves(save, events);
  const model = new BoardModel({
    items,
    reload: () => session.reload(),
    notify: (n) => notice.show(noticeText(n)),
  });
  events.saved = (key, item) => model.saved(key, item);
  events.rejected = (key, e) => model.rejected(key, e);
  // svelte-ignore state_referenced_locally
  tracker.items = model;
  $effect(() => () => {
    if (tracker.items === model) tracker.items = null;
  });

  // 読み込んだボードを出す。日を移ったら、保存を待っていない項目の待ち行列を片付ける
  $effect(() => {
    const b = session.board;
    if (b === null) return;
    untrack(() => {
      if (model.date !== null && model.date !== b.date) items.prune();
      model.load(b);
    });
  });

  const layout = $derived<Layout>(narrow ? "C" : prefs.layout);
  const isToday = $derived(model.date !== null && model.date === session.today);

  // ---- フォーカス ----

  let focusReq = $state<(FocusTarget & { n: number }) | null>(null);
  let seq = 0;
  function requestFocus(target: FocusTarget | null) {
    if (target !== null) focusReq = { ...target, n: ++seq };
  }

  let todoRegion = $state<HTMLElement | null>(null);
  let blockerRegion = $state<HTMLElement | null>(null);
  let editor = $state<EditorView | null>(null);
  /** 領域ごとに、最後に選んでいた行（Mod+1 / 2 で戻る先）。 */
  const lastIn: Record<ItemKind, string | null> = { todo: null, blocker: null };
  $effect(() => {
    const f = model.focus;
    if (f === null) return;
    const row = untrack(() => model.row(f.key));
    if (row !== undefined) lastIn[row.kind] = row.key;
  });

  function focusRegion(kind: ItemKind | "reflection") {
    if (kind === "reflection") {
      editor?.focus();
      return;
    }
    const rows = model.list(kind);
    const row = rows.find((r) => r.key === lastIn[kind]) ?? rows.find((r) => r.status === "open") ?? rows[0];
    if (row === undefined) (kind === "todo" ? todoRegion : blockerRegion)?.focus();
    else requestFocus({ key: row.key, field: "text", offset: "end" });
  }

  // Vim の挿入モード・続きのキー待ちは、Vim モードを切り替えたとき、日を移ったときに捨てる
  $effect(() => {
    void prefs.vimMode;
    void model.date;
    untrack(() => {
      model.insert = false;
      model.vimPending = null;
    });
  });

  // Vim の挿入モードの出入りに合わせて、IME を切り替える（操作仕様書 4.4。挿入モードを抜ける経路は
  // Esc・フォーカスの移動・日の移動などいくつもあるので、出入りの変化で見る）。Vimモードを無効にしたことで
  // 挿入モードが終わったときは、切り替えない（`imeLeaveInsert` が Vimモードの有効を確かめる）
  let wasInsert = false;
  $effect(() => {
    const insert = model.insert;
    untrack(() => {
      if (insert && !wasInsert) imeEnterInsert();
      else if (!insert && wasInsert) imeLeaveInsert();
      wasInsert = insert;
    });
  });

  /** フォーカスが一覧の外（サイドバー・振り返りの欄など）へ移ったら、挿入モードを終える（戻ったとき、項目を選ぶだけにするため）。 */
  function onFocusOut(e: FocusEvent) {
    const next = e.relatedTarget;
    // フォーカスの行き先が無いとき：窓が前面から外れた（Alt+Tab など）なら、挿入モードを続ける。窓の中の
    // フォーカスできない所を押したのなら、終える
    const outside = next instanceof Element ? next.closest(".r-todo, .r-blk") === null : next === null && document.hasFocus();
    if (outside) {
      model.insert = false;
      model.vimPending = null;
    }
  }

  /** 領域 `from` から前（-1）・次（+1）の領域へ。TODO・障害・振り返りの順に回る（Vim の Tab・Shift+Tab）。 */
  function moveRegion(from: ItemKind | "reflection", dir: -1 | 1) {
    const order = ["todo", "blocker", "reflection"] as const;
    focusRegion(order[(order.indexOf(from) + dir + order.length) % order.length]);
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.defaultPrevented) return;
    const a = boardActionOf(e, mac);
    if (a === "regionTodo" || a === "regionBlocker" || a === "regionReflection") {
      e.preventDefault();
      focusRegion(a === "regionTodo" ? "todo" : a === "regionBlocker" ? "blocker" : "reflection");
    }
  }

  // 持ち越し先の印・検索パレットの項目の結果：その日を開いて、その項目を選ぶ。持ち越し先の印からは、
  // 項目が一覧に入るまで待つ（読み直しと行き違って、まだ入っていない一覧が先に届くことがあるため）。
  // パレットの結果からは、その日を読み終えて項目が無ければ（検索の後に消されたなど）、その日を開くだけに
  // する（`mustExist`。後で戻されたときに、思わぬところでフォーカスを動かさないため。画面設計書 6章）。
  // どちらも、選ぶ前にほかの日へ移ったら、選ぶのをやめる
  let pendingSelect = $state<{ date: LogicalDate; id: Id; mustExist?: boolean } | null>(null);
  function openCarried(to: CarriedTo) {
    pendingSelect = { date: to.date, id: to.id };
    session.show(to.date);
  }
  $effect(() => {
    const showing = session.showing;
    untrack(() => {
      if (pendingSelect !== null && showing !== pendingSelect.date) pendingSelect = null;
    });
  });
  $effect(() => {
    const d = model.date;
    const p = pendingSelect;
    void model.todos;
    void model.blockers;
    untrack(() => {
      if (p === null || d !== p.date) return;
      const row = model.todos.find((r) => r.id === p.id) ?? model.blockers.find((r) => r.id === p.id);
      if (row === undefined) {
        // 読み終えた（表示しようとしている日と表示している日がそろった）一覧に無ければ、やめる
        if (p.mustExist && session.showing === session.date) pendingSelect = null;
        return;
      }
      pendingSelect = null;
      requestFocus({ key: row.key, field: "text", offset: "end" });
    });
  });

  // 検索パレットの結果（日を開くのと、モードを整理モードにするのは App）
  let lastTarget = 0;
  $effect(() => {
    const tg = target;
    if (tg === null || tg.n === lastTarget) return;
    lastTarget = tg.n;
    untrack(() => {
      if (!("itemId" in tg)) return;
      // 移る先は Board が持つので、App の移る先はここで終える（その日を開くのは App）
      pendingSelect = { date: tg.date, id: tg.itemId, mustExist: true };
      onTargetDone();
    });
  });
  /** 振り返りの中で移る先（その日の振り返りを読み終えたら、ReflectionEditor が移る）。 */
  const reflectionFind = $derived(target !== null && "find" in target ? { date: target.date, text: target.find, n: target.n } : null);

  // ---- 入力中の状態 ----

  const activity = () => tracker.activity();
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="board" data-layout={layout} data-date={model.date ?? ""} onkeydown={onKeydown} onfocusout={onFocusOut}>
  {#if model.date !== null}
    <ItemList
      kind="todo"
      {model}
      {mac}
      {isToday}
      reviewAt={prefs.reviewThreshold}
      {focusReq}
      onFocus={requestFocus}
      onRegion={moveRegion}
      onOpenCarried={openCarried}
      {onOpenNoteSource}
      onActivity={activity}
      bind:region={todoRegion}
    />
    <ItemList
      kind="blocker"
      {model}
      {mac}
      {isToday}
      reviewAt={prefs.reviewThreshold}
      {focusReq}
      onFocus={requestFocus}
      onRegion={moveRegion}
      onOpenCarried={openCarried}
      {onOpenNoteSource}
      onActivity={activity}
      bind:region={blockerRegion}
    />
    <section class="region r-ref" data-region="reflection" aria-label={m.reflectionLabel}>
      <div class="rh"><h2 class="label">{m.labels.region.reflection}</h2></div>
      <div class="scroll">
        <ReflectionEditor
          date={model.date}
          {save}
          presence={tracker.reflection}
          rev={session.board?.reflectionRev ?? 0}
          onActivity={activity}
          onRegion={(dir) => moveRegion("reflection", dir)}
          find={reflectionFind}
          onFound={onTargetDone}
          bind:view={editor}
        />
      </div>
    </section>
  {/if}
</div>

<style>
  .board {
    position: relative;
    height: 100%;
    display: grid;
    column-gap: calc(56 * var(--u));
    row-gap: calc(24 * var(--u));
    padding: calc(24 * var(--u)) var(--pad) calc(8 * var(--u));
  }

  /* A. 2段：左に TODO（全高）、右上に障害（内容の高さ）、右下に振り返り（残り）。列は 1.15 : 1 */
  .board[data-layout="A"] {
    grid-template-columns: minmax(0, 1.15fr) minmax(0, 1fr);
    grid-template-rows: auto minmax(0, 1fr);
    grid-template-areas: "todo blk" "todo ref";
  }

  /* B. 3列 */
  .board[data-layout="B"] {
    grid-template-columns: repeat(3, minmax(0, 1fr));
    grid-template-rows: minmax(0, 1fr);
    grid-template-areas: "todo blk ref";
  }

  /* C. 縦積み：本文ごと縦に流す */
  .board[data-layout="C"] {
    grid-template-columns: minmax(0, 1fr);
    grid-template-areas: "todo" "blk" "ref";
    align-content: start;
    overflow: auto;
  }

  .board :global(.r-todo) {
    grid-area: todo;
  }

  .board :global(.r-blk) {
    grid-area: blk;
  }

  .r-ref {
    grid-area: ref;
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
  }

  /* 領域の区切りは振り返りの上の罫線だけ（A）。縦積みは障害と振り返りの上 */
  .board[data-layout="A"] .r-ref,
  .board[data-layout="C"] .r-ref,
  .board[data-layout="C"] :global(.r-blk) {
    border-top: 1px solid var(--rule);
    padding-top: calc(18 * var(--u));
  }

  .board[data-layout="C"] :global(.scroll) {
    overflow: visible;
  }

  .rh {
    display: flex;
    align-items: baseline;
    gap: 14px;
    margin: 0 0 calc(8 * var(--u));
    flex: none;
  }

  .rh h2 {
    margin: 0;
  }

  .scroll {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: auto;
    scrollbar-width: thin;
    scrollbar-color: var(--rule) transparent;
  }
</style>
