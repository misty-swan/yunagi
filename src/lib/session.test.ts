import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync } from "svelte";
import { afterEach, expect, test, vi } from "vitest";
import { EVENTS, type Board, type Overview, type Startup } from "./bindings";
import { core, CoreError } from "./core";
import { RETRY_MS, startSession, type Session } from "./session.svelte";

let session: Session | undefined;

afterEach(async () => {
  await session?.stop();
  session = undefined;
  clearMocks();
});

function board(date: string, text: string): Board {
  return {
    date,
    todos: [
      {
        item: {
          id: "01JC0000000000000000000000",
          date,
          kind: "todo",
          text,
          status: "open",
          closeReason: null,
          sortKey: "a0",
          carryCount: 0,
          carriedFromId: null,
          splitFromId: null,
          resolutionNote: null,
          origin: "human",
          agentClientId: null,
          externalRefId: null,
          completedAt: null,
          createdAt: 0,
          updatedAt: 0,
          deletedAt: null,
        },
        carriedTo: null,
      },
    ],
    blockers: [],
    reflection: "",
    note: { headings: [], length: 0 },
  };
}

function overview(today: string): Overview {
  const counts = { todos: 1, done: 0, leftTodos: 1, leftBlockers: 0, resolved: 0, carried: 0 };
  return { today, counts, recent: [{ date: today, counts }], unseenChanges: 0 };
}

/** 模擬のコアの今日（テストの中で変えると、次の問い合わせから変わる）。 */
const clock = { today: "2026-11-03" };

/** 模擬のコア。`texts` は日付ごとのボードの TODO の本文（変えると次の読み込みに出る）。 */
function mockCore(status: Startup, texts: Record<string, string>) {
  clock.today = "2026-11-03";
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC(
    (cmd, args) => {
      calls.push({ cmd, args });
      switch (cmd) {
        case "startup_status":
          return status;
        case "today":
          return clock.today;
        case "get_board": {
          const date = (args as { date: string }).date;
          return board(date, texts[date] ?? "");
        }
        case "get_overview":
          return overview(clock.today);
      }
      throw new Error(`unexpected command ${cmd}`);
    },
    { shouldMockEvents: true },
  );
  return calls;
}

/** 状態が条件に合うまで待つ。 */
async function until(check: () => boolean) {
  await vi.waitFor(() => {
    flushSync();
    expect(check()).toBe(true);
  });
}

test("開いていれば今日のボードを読み、変わるたびに読み込み直す", async () => {
  const texts: Record<string, string> = { "2026-11-03": "設計を読む" };
  const calls = mockCore({ state: "ready" }, texts);
  session = startSession();
  const s = session;
  await until(() => s.board !== null);
  expect(s.startup).toEqual({ state: "ready" });
  expect(s.today).toBe("2026-11-03");
  expect(s.board?.todos[0].item.text).toBe("設計を読む");

  // エージェントの変更（changed）で読み込み直す
  texts["2026-11-03"] = "エージェントが変えた";
  await emit(EVENTS.changed, { by: "agent" });
  await until(() => s.board?.todos[0].item.text === "エージェントが変えた");

  // 日付が変わったら、新しい日を読む
  texts["2026-11-04"] = "翌日";
  clock.today = "2026-11-04";
  await emit(EVENTS.todayChanged, { today: "2026-11-04" });
  await until(() => s.today === "2026-11-04");
  expect(s.board?.date).toBe("2026-11-04");
  expect(calls.filter((c) => c.cmd === "get_board").map((c) => c.args)).toEqual([
    { date: "2026-11-03" },
    { date: "2026-11-03" },
    { date: "2026-11-04" },
  ]);
});

test("ほかの日を表示し、その日のまま読み込み直す。今日を表示していれば新しい今日へ移る", async () => {
  const texts: Record<string, string> = { "2026-11-03": "今日", "2026-11-01": "前の日" };
  const calls = mockCore({ state: "ready" }, texts);
  session = startSession();
  const s = session;
  await until(() => s.date === "2026-11-03");
  expect(s.overview?.today).toBe("2026-11-03");

  s.show("2026-11-01");
  await until(() => s.date === "2026-11-01");
  expect(s.board?.todos[0].item.text).toBe("前の日");
  expect(s.today).toBe("2026-11-03");

  // ほかの日を表示している間に日付が変わっても、その日のまま（今日と Recent だけ変わる）
  clock.today = "2026-11-04";
  await emit(EVENTS.todayChanged, { today: "2026-11-04" });
  await until(() => s.overview?.today === "2026-11-04");
  expect(s.date).toBe("2026-11-01");
  expect(s.today).toBe("2026-11-04");

  // 今日を表示していれば、新しい今日へ移る
  s.show("2026-11-04");
  await until(() => s.date === "2026-11-04");
  clock.today = "2026-11-05";
  await emit(EVENTS.todayChanged, { today: "2026-11-05" });
  await until(() => s.date === "2026-11-05");
  const overviews = calls.filter((c) => c.cmd === "get_overview").map((c) => c.args);
  expect(overviews).toEqual([
    { date: "2026-11-03" },
    { date: "2026-11-01" },
    { date: "2026-11-01" },
    { date: "2026-11-04" },
    { date: "2026-11-05" },
  ]);
});

test("開く前に表示する日を決めたら、開いてからその日を読む", async () => {
  const calls = mockCore({ state: "opening" }, { "2026-10-30": "決めた日" });
  session = startSession();
  const s = session;
  await until(() => s.startup?.state === "opening");
  s.show("2026-10-30");
  await emit(EVENTS.startup, { state: "ready" });
  await until(() => s.date === "2026-10-30");
  expect(s.board?.todos[0].item.text).toBe("決めた日");
  expect(calls.some((c) => c.cmd === "today")).toBe(false);
});

test("開いている途中なら、startup が届いてから読む", async () => {
  const calls = mockCore({ state: "opening" }, { "2026-11-03": "x" });
  session = startSession();
  const s = session;
  await until(() => s.startup?.state === "opening");
  // 開く前の changed では読まない
  await emit(EVENTS.changed, { by: "screen" });
  expect(calls.some((c) => c.cmd === "get_board")).toBe(false);

  await emit(EVENTS.startup, { state: "ready" });
  await until(() => s.board !== null);
  expect(s.board?.todos[0].item.text).toBe("x");
});

test("見つからないときは、探した場所を持つ", async () => {
  const notFound: Startup = {
    state: "not_found",
    path: "/mnt/sync/yunagi.db",
    dataDir: "/mnt/sync",
    configuredIn: "/home/u/.local/share/yunagi/config.json",
    dataDirExists: false,
  };
  mockCore(notFound, {});
  session = startSession();
  const s = session;
  await until(() => s.startup !== null);
  expect(s.startup).toEqual(notFound);
  expect(s.board).toBeNull();
});

test("コマンドの失敗は CoreError で、コアのエラーの形をそのまま持つ", async () => {
  mockIPC((cmd) => {
    if (cmd === "save_item") {
      throw { kind: "invalid_argument", reason: "empty_text" };
    }
    if (cmd === "move_item") {
      // 引数の形の誤り（Tauri が読めなかったとき）は文字列で返る
      throw "invalid args `id` for command `move_item`";
    }
  });
  const e1 = await core.saveItem("01JC0000000000000000000000", { text: "" }).catch((e) => e);
  expect(e1).toBeInstanceOf(CoreError);
  expect(e1.error).toEqual({ kind: "invalid_argument", reason: "empty_text" });
  const e2 = await core.moveItem("x", { at: "first" }).catch((e) => e);
  expect(e2.error.kind).toBe("bad_request");
});

test("コマンドの引数は camelCase で渡す", async () => {
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    return { result: "saved", rev: 2, updated: true };
  });
  await core.saveTextDoc("note", "2026-11-03", "本文", 1);
  await core.saveItem("01JC0000000000000000000000", {
    status: { before: "open", closedAt: 1700000000000, changedAt: 1700000000000 },
  });
  expect(calls).toEqual([
    {
      cmd: "save_text_doc",
      args: { doc: "note", date: "2026-11-03", body: "本文", baseRev: 1 },
    },
    {
      cmd: "save_item",
      args: {
        id: "01JC0000000000000000000000",
        fields: { status: { before: "open", closedAt: 1700000000000, changedAt: 1700000000000 } },
      },
    },
  ]);
});

test("新しい日を読んでいる途中に changed が届いても、前の日に戻らない", async () => {
  const texts: Record<string, string> = { "2026-11-03": "今日", "2026-11-04": "翌日" };
  let release: (() => void) | undefined;
  mockIPC(
    async (cmd, args) => {
      if (cmd === "startup_status") return { state: "ready" };
      if (cmd === "today") return "2026-11-03";
      if (cmd === "get_board") {
        const date = (args as { date: string }).date;
        // 翌日の1回目の読み込みを止めておく
        if (date === "2026-11-04" && release === undefined) {
          await new Promise<void>((r) => (release = r));
        }
        return board(date, texts[date]);
      }
      if (cmd === "get_overview") return overview((args as { date: string }).date);
    },
    { shouldMockEvents: true },
  );
  session = startSession();
  const s = session;
  await until(() => s.board?.date === "2026-11-03");
  await emit(EVENTS.todayChanged, { today: "2026-11-04" });
  await vi.waitFor(() => expect(release).toBeDefined());
  await emit(EVENTS.changed, { by: "agent" });
  release?.();
  await until(() => s.board?.date === "2026-11-04");
  expect(s.today).toBe("2026-11-04");
});

test("開く処理の状態の問い合わせに失敗したら、問い合わせ直す", async () => {
  vi.useFakeTimers();
  try {
    let asks = 0;
    mockIPC(
      (cmd) => {
        if (cmd === "startup_status") {
          asks += 1;
          if (asks === 1) throw { kind: "internal", detail: "x" };
          return { state: "ready" };
        }
        if (cmd === "today") return "2026-11-03";
        if (cmd === "get_board") return board("2026-11-03", "x");
        if (cmd === "get_overview") return overview("2026-11-03");
      },
      { shouldMockEvents: true },
    );
    session = startSession();
    const s = session;
    await vi.waitFor(() => expect(s.error?.kind).toBe("internal"));
    await vi.advanceTimersByTimeAsync(RETRY_MS);
    await vi.waitFor(() => {
      flushSync();
      expect(s.board).not.toBeNull();
    });
    expect(asks).toBe(2);
    expect(s.error).toBeNull();
  } finally {
    vi.useRealTimers();
  }
});

test("イベントを聞き始めるのに失敗したら、やり直してから状態を問い合わせる", async () => {
  vi.useFakeTimers();
  try {
    let listens = 0;
    const calls: string[] = [];
    mockIPC((cmd) => {
      calls.push(cmd);
      if (cmd === "plugin:event|listen") {
        listens += 1;
        if (listens === 1) throw "listen failed";
        return listens;
      }
      if (cmd === "plugin:event|unlisten") return null;
      if (cmd === "startup_status") return { state: "ready" };
      if (cmd === "today") return "2026-11-03";
      if (cmd === "get_board") return board("2026-11-03", "x");
      if (cmd === "get_overview") return overview("2026-11-03");
    });
    session = startSession();
    const s = session;
    await vi.waitFor(() => expect(s.error?.kind).toBe("bad_request"));
    expect(calls).not.toContain("startup_status");
    await vi.advanceTimersByTimeAsync(RETRY_MS);
    await vi.waitFor(() => {
      flushSync();
      expect(s.board).not.toBeNull();
    });
    // 3つのイベントを聞き、1つはやり直した
    expect(listens).toBe(4);
  } finally {
    vi.useRealTimers();
  }
});

test("入力中の状態には、読み込みの印と増える番号を付ける", async () => {
  const sent: { session: string; seq: number }[] = [];
  mockIPC((cmd, args) => {
    if (cmd === "report_ui_state") {
      sent.push((args as { state: { session: string; seq: number } }).state);
      return null;
    }
  });
  await core.reportUiState({ editingItems: [], editors: [] });
  await core.reportUiState({ editingItems: [], editors: [] });
  expect(sent[0].session).not.toBe("");
  expect(sent[1].session).toBe(sent[0].session);
  expect(sent[1].seq).toBe(sent[0].seq + 1);
});
