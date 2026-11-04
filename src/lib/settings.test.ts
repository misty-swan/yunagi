// 設定の保存（src/lib/settings.svelte.ts）：失敗の巻き戻しが、あとから変えた新しい値を上書きしない。
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, test } from "vitest";
import { prefs } from "./prefs.svelte";
import { SettingsStore } from "./settings.svelte";

afterEach(() => {
  clearMocks();
  prefs.fontSize = 15;
  prefs.theme = "system";
});

test("続けて同じ欄を変え、先の保存だけ失敗しても、あとの新しい値は戻さない", async () => {
  mockIPC((cmd, args) => {
    if (cmd !== "save_settings") return null;
    if ((args as { patch: { fontSize?: number } }).patch.fontSize === 17) throw { kind: "storage", detail: "x" };
    return null;
  });
  const store = new SettingsStore();
  const first = store.save({ fontSize: 17 });
  const second = store.save({ fontSize: 18 });
  expect(prefs.fontSize).toBe(18);
  expect(await first).toBe(false);
  expect(await second).toBe(true);
  expect(prefs.fontSize).toBe(18);
});

test("ほかの欄の新しい保存があっても、失敗した欄は戻し、silent なら失敗を持たない", async () => {
  mockIPC(() => {
    throw { kind: "storage", detail: "x" };
  });
  const store = new SettingsStore();
  await store.save({ theme: "dark" }, { silent: true });
  expect(prefs.theme).toBe("system");
  expect(store.error).toBeNull();
  await store.save({ theme: "dark" });
  expect(prefs.theme).toBe("system");
  expect(store.error?.kind).toBe("storage");
});
