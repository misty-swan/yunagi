// ふりかえり（S-08）の状態：読み込みの順序、行き違い、日と週の移動（今日で止まる）。コアは模擬。
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { ArchiveScreen } from "./archive.svelte";
import type { Archive, LogicalDate } from "./bindings";

const TODAY = "2026-11-04";

function archiveOf(date: LogicalDate | null): Archive {
  const zero = { todos: 0, done: 0, leftTodos: 0, leftBlockers: 0, resolved: 0, carried: 0 };
  const sel = date === null ? "2026-11-03" : date > TODAY ? TODAY : date;
  return {
    date: sel,
    today: TODAY,
    week: [],
    weekTotal: zero,
    daily: [],
    preview: { counts: zero, reflection: "", note: [] },
  };
}

const log: string[] = [];
const gates: Record<string, () => void> = {};
let failing = false;

beforeEach(() => {
  log.length = 0;
  failing = false;
  mockIPC(async (cmd, args) => {
    if (cmd !== "get_archive") return undefined;
    const date = (args as { date: string | null }).date;
    log.push(`get:${date}`);
    // 日付ごとに、答えを返すのを待たせられる
    if (date !== null && date in gates) await new Promise<void>((resolve) => (gates[date] = resolve));
    if (failing) throw { kind: "storage", message: "x" };
    return archiveOf(date);
  });
});

afterEach(() => {
  for (const k of Object.keys(gates)) delete gates[k];
  clearMocks();
});

function screen(flush: () => Promise<void> = async () => void log.push("flush")) {
  return new ArchiveScreen({ today: () => TODAY, flush });
}

test("読む前に、保存していない入力を送る。送れなくても読む", async () => {
  const s = screen();
  s.show(null);
  await vi.waitFor(() => expect(s.data?.date).toBe("2026-11-03"));
  expect(log).toEqual(["flush", "get:null"]);
  expect(s.sel).toBe("2026-11-03");

  const failed = screen(async () => {
    throw new Error("save failed");
  });
  failed.show("2026-11-02");
  await vi.waitFor(() => expect(failed.data?.date).toBe("2026-11-02"));
  failed.close();
});

test("続けて選んだら、最後に選んだ日の答えだけを使う（古い答えが後から届いても）", async () => {
  const s = screen();
  s.show("2026-11-01");
  await vi.waitFor(() => expect(s.data?.date).toBe("2026-11-01"));
  gates["2026-11-02"] = () => {};
  s.select("2026-11-02");
  await vi.waitFor(() => expect(log.at(-1)).toBe("get:2026-11-02"));
  s.select("2026-11-03");
  await vi.waitFor(() => expect(s.data?.date).toBe("2026-11-03"));
  // 止めていた古い答えを返す
  gates["2026-11-02"]();
  await new Promise((resolve) => setTimeout(resolve, 20));
  expect(s.data?.date).toBe("2026-11-03");
  expect(s.sel).toBe("2026-11-03");
});

test("日は 1 日ずつ、週は 7 日ずつ動き、今日で止まる。今日より後は選べない", async () => {
  const s = screen();
  s.show("2026-11-03");
  await vi.waitFor(() => expect(s.data?.date).toBe("2026-11-03"));
  s.moveDay(1);
  await vi.waitFor(() => expect(s.data?.date).toBe(TODAY));
  const n = log.length;
  s.moveDay(1);
  s.select("2026-11-05");
  expect(log.length).toBe(n);
  s.moveWeek(1);
  expect(log.length).toBe(n);
  s.moveWeek(-1);
  await vi.waitFor(() => expect(s.data?.date).toBe("2026-10-28"));
  s.moveDay(-1);
  await vi.waitFor(() => expect(s.data?.date).toBe("2026-10-27"));
  // 開く前に今日より後を指定しても、今日にする
  s.close();
  s.show("2026-12-01");
  expect(s.sel).toBe(TODAY);
});

test("閉じたら、待っていた答えは使わない。閉じている間は動かさない", async () => {
  const s = screen();
  gates["2026-11-02"] = () => {};
  s.show("2026-11-02");
  await vi.waitFor(() => expect(log.at(-1)).toBe("get:2026-11-02"));
  s.close();
  gates["2026-11-02"]();
  await new Promise((resolve) => setTimeout(resolve, 20));
  expect(s.open).toBe(false);
  expect(s.data).toBeNull();
  const n = log.length;
  s.moveDay(1);
  s.select("2026-11-03");
  expect(log.length).toBe(n);
});

test("読めなければ error を持ち、読めるまで読み直す", async () => {
  failing = true;
  const s = screen();
  s.show("2026-11-02");
  await vi.waitFor(() => expect(s.error?.kind).toBe("storage"));
  expect(s.data).toBeNull();
  failing = false;
  await vi.waitFor(() => expect(s.data?.date).toBe("2026-11-02"), { timeout: 3000 });
  expect(s.error).toBeNull();
  s.stop();
});
