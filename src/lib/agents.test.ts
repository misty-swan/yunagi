// エージェント変更履歴（S-06）の状態と、エージェントの一覧（AgentAdmin）：読み込みの順序、確かめる前に入力を送ること、
// 変わったときの読み直し。コアは模擬。
import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { AgentAdmin } from "./agent_admin.svelte";
import { AgentsScreen } from "./agents.svelte";
import { EVENTS, type AgentRow, type ChangeEntry, type RevertPlan } from "./bindings";

const ENTRY: ChangeEntry = {
  id: "01J000000000000000000000E1",
  createdAt: 1793600000000,
  actor: "agent",
  client: "claude-code",
  operation: "todo.edit",
  target: { type: "item", id: "01J0000000000000000000ITEM", item: null },
  summary: { type: "unknown" },
  seenAt: null,
  revertedAt: null,
  revertOf: null,
};

const PLAN: RevertPlan = {
  id: ENTRY.id,
  operation: "todo.edit",
  client: "claude-code",
  createdAt: 1793600000000,
  state: "apply",
  token: "t1",
  changedSince: [],
  effect: { type: "none" },
};

const ROWS: AgentRow[] = [
  { id: "01J00000000000000000000001", name: "claude-code", scopes: ["read"], createdAt: 1, lastSeenAt: null, removable: false },
];

const log: string[] = [];

beforeEach(() => {
  log.length = 0;
  mockIPC(
    async (cmd, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      log.push(cmd === "list_changes" ? `list_changes:${a.limit}` : cmd);
      switch (cmd) {
        case "list_changes":
          return { changes: [ENTRY], nextBefore: null };
        case "preview_revert":
          return PLAN;
        case "list_agents":
          return ROWS;
      }
    },
    { shouldMockEvents: true },
  );
});

afterEach(() => clearMocks());

test("取り消しの確認は、まだ保存していない入力を送り終えてから、取り消す内容を読む", async () => {
  let release: () => void = () => {};
  const flushed = new Promise<void>((resolve) => (release = resolve));
  const s = new AgentsScreen({
    flush: async () => {
      log.push("flush");
      await flushed;
    },
  });
  s.show();
  await vi.waitFor(() => expect(s.loaded).toBe(true));
  const started = s.startRevert(ENTRY);
  await new Promise((resolve) => setTimeout(resolve, 20));
  // 送り終えるまで、取り消す内容は読まない
  expect(log).toContain("flush");
  expect(log).not.toContain("preview_revert");
  release();
  await started;
  expect(s.flow?.plan?.token).toBe("t1");
  expect(log.indexOf("flush")).toBeLessThan(log.indexOf("preview_revert"));
  s.stop();
});

test("入力を送れなかった（保存に失敗している）ときも、取り消す内容は読む", async () => {
  const s = new AgentsScreen({
    flush: async () => {
      throw new Error("save failed");
    },
  });
  s.show();
  await vi.waitFor(() => expect(s.loaded).toBe(true));
  await s.startRevert(ENTRY);
  expect(s.flow?.plan?.token).toBe("t1");
  s.stop();
});

test("開いている間だけ、変更が届くたびに読み込み済みの件数で読み直す。閉じたら読み直さない", async () => {
  const s = new AgentsScreen({ flush: async () => {} });
  s.start();
  s.show();
  await vi.waitFor(() => expect(s.loaded).toBe(true));
  // 読み込みの件数は 50 から。changed を聞き始めるまで待つ
  await vi.waitFor(async () => {
    log.length = 0;
    await emit(EVENTS.changed, { by: "agent" });
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(log).toEqual(["list_changes:50"]);
  });
  s.close();
  log.length = 0;
  await emit(EVENTS.changed, { by: "agent" });
  await new Promise((resolve) => setTimeout(resolve, 20));
  expect(log).toEqual([]);
  s.stop();
});

test("設定の onBack は、利用者が閉じたときだけ働く。画面の都合で閉じたときは捨てる", async () => {
  const s = new AgentsScreen({ flush: async () => {} });
  let back = 0;
  s.show();
  s.onBack = () => back++;
  s.dismiss();
  expect(back).toBe(1);
  expect(s.open).toBe(false);
  s.show();
  s.onBack = () => back++;
  s.close();
  expect(s.onBack).toBeNull();
  s.dismiss();
  expect(back).toBe(1);
  s.stop();
});

test("エージェントの一覧は、出している間だけ、変更が届くたびに読み直す", async () => {
  const admin = new AgentAdmin();
  admin.start();
  await admin.load();
  const release = admin.retain();
  await vi.waitFor(async () => {
    log.length = 0;
    await emit(EVENTS.changed, { by: "agent" });
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(log).toEqual(["list_agents"]);
  });
  release();
  release(); // 2 回呼んでも、数が狂わない
  // 出し終えたら読み直さない（自動保存のたびに読み続けない）
  log.length = 0;
  await emit(EVENTS.changed, { by: "agent" });
  await new Promise((resolve) => setTimeout(resolve, 20));
  expect(log).toEqual([]);
  admin.stop();
});
