// 自動更新の状態と「更新して再起動」：イベントと問い合わせの順序、保存してから始める、保存できていなければ
// 始めない。コアは模擬。
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { UpdateState } from "./bindings";
import { UpdateStore } from "./update.svelte";

const available: UpdateState = { state: "available", version: "1.2.0", notes: "修正" };

let calls: string[];
let current: UpdateState;
let emit: ((s: UpdateState) => void) | null;

beforeEach(() => {
  calls = [];
  current = { state: "none" };
  emit = null;
  mockIPC(
    (cmd, args) => {
      calls.push(cmd);
      if (cmd === "get_update_state") return current;
      if (cmd === "plugin:event|listen") {
        const handler = (args as { handler: number }).handler;
        emit = (s) => {
          // イベントの届け方は @tauri-apps/api の模擬に合わせる
          (window as unknown as { __TAURI_INTERNALS__: { runCallback(id: number, data: unknown): void } })
            .__TAURI_INTERNALS__.runCallback(handler, { event: "update-state", payload: s });
        };
        return handler;
      }
      return null;
    },
    { shouldMockEvents: false },
  );
});

afterEach(() => clearMocks());

function store(deps: Partial<ConstructorParameters<typeof UpdateStore>[0]> = {}) {
  const s = new UpdateStore({ flush: async () => {}, saveFailing: () => false, ...deps });
  s.start();
  return s;
}

test("聞き始めてから現在の状態を問い合わせ、見つかっていれば通知を出す", async () => {
  current = available;
  const s = store();
  await vi.waitFor(() => expect(s.visible).toBe(true));
  expect(calls.indexOf("plugin:event|listen")).toBeLessThan(calls.indexOf("get_update_state"));
  expect(s.version).toBe("1.2.0");
  expect(s.notes).toBe("修正");
  expect(s.canInstall).toBe(true);
  s.stop();
});

test("新しい版が無いときは何も出さず、更新して再起動も始められない", async () => {
  const s = store();
  await vi.waitFor(() => expect(calls).toContain("get_update_state"));
  expect(s.visible).toBe(false);
  expect(s.canInstall).toBe(false);
  await s.install();
  expect(calls).not.toContain("install_update");
  s.stop();
});

test("コアのイベントで、ダウンロードの割合・失敗・再起動に変わる", async () => {
  const s = store();
  await vi.waitFor(() => expect(emit).not.toBeNull());
  emit!({ state: "downloading", version: "1.2.0", progress: 40 });
  expect(s.progress).toBe(40);
  expect(s.canInstall).toBe(false);
  emit!({ state: "failed", version: "1.2.0" });
  expect(s.visible).toBe(true);
  expect(s.canInstall).toBe(true);
  expect(s.progress).toBeUndefined();
  emit!({ state: "none" });
  expect(s.visible).toBe(false);
  s.stop();
});

test("更新して再起動は、入力中の内容を保存し終えてからコアに頼む", async () => {
  current = available;
  const order: string[] = [];
  const s = store({
    flush: async () => {
      order.push("flush");
    },
  });
  await vi.waitFor(() => expect(s.canInstall).toBe(true));
  await s.install();
  order.push(calls.includes("install_update") ? "install" : "none");
  expect(order).toEqual(["flush", "install"]);
  s.stop();
});

test("保存し終えられなかった（flush が失敗した）ときも、更新を始めず、断った表示を出す", async () => {
  current = available;
  const s = store({
    flush: async () => {
      throw new Error("ipc");
    },
  });
  await vi.waitFor(() => expect(s.canInstall).toBe(true));
  await s.install();
  expect(s.refused).toBe(true);
  expect(calls).not.toContain("install_update");
  s.stop();
});

test("保存できていないときは、更新を始めない（refused）", async () => {
  current = available;
  const s = store({ saveFailing: () => true });
  await vi.waitFor(() => expect(s.canInstall).toBe(true));
  await s.install();
  expect(s.refused).toBe(true);
  expect(calls).not.toContain("install_update");
  // 状態が変わったら、断った表示は消える
  emit!(available);
  expect(s.refused).toBe(false);
  s.stop();
});
