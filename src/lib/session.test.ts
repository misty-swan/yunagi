import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync } from "svelte";
import { afterEach, expect, test, vi } from "vitest";
import { EVENTS, type Board, type Startup } from "./bindings";
import { core, CoreError } from "./core";
import { startSession, type Session } from "./session.svelte";

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

/** 模擬のコア。`texts` は日付ごとのボードの TODO の本文（変えると次の読み込みに出る）。 */
function mockCore(status: Startup, texts: Record<string, string>, today = "2026-11-03") {
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC(
    (cmd, args) => {
      calls.push({ cmd, args });
      switch (cmd) {
        case "startup_status":
          return status;
        case "today":
          return today;
        case "get_board": {
          const date = (args as { date: string }).date;
          return board(date, texts[date] ?? "");
        }
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
  await emit(EVENTS.todayChanged, { today: "2026-11-04" });
  await until(() => s.today === "2026-11-04");
  expect(s.board?.date).toBe("2026-11-04");
  expect(calls.filter((c) => c.cmd === "get_board").map((c) => c.args)).toEqual([
    { date: "2026-11-03" },
    { date: "2026-11-03" },
    { date: "2026-11-04" },
  ]);
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
    if (cmd === "save_item_text") {
      throw { kind: "invalid_argument", reason: "empty_text" };
    }
    if (cmd === "move_item") {
      // 引数の形の誤り（Tauri が読めなかったとき）は文字列で返る
      throw "invalid args `id` for command `move_item`";
    }
  });
  const e1 = await core.saveItemText("01JC0000000000000000000000", "").catch((e) => e);
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
  await core.setItemClosed("01JC0000000000000000000000", 1700000000000);
  expect(calls).toEqual([
    {
      cmd: "save_text_doc",
      args: { doc: "note", date: "2026-11-03", body: "本文", baseRev: 1 },
    },
    {
      cmd: "set_item_closed",
      args: { id: "01JC0000000000000000000000", closedAt: 1700000000000 },
    },
  ]);
});
