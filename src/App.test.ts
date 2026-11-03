import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, expect, test, vi } from "vitest";
import App from "./App.svelte";

let app: ReturnType<typeof mount> | undefined;

// テストが失敗しても、描いた画面を必ず片付ける
afterEach(async () => {
  if (app) unmount(app);
  app = undefined;
  document.body.innerHTML = "";
  // 片付けで、コアのイベントを聞くのをやめ終わってから（次のタスクまで待つ）、模擬を外す
  await new Promise((resolve) => setTimeout(resolve, 0));
  clearMocks();
});

test("コアとつなぎ、開く処理の状態を出す", async () => {
  mockIPC(
    (cmd) => {
      if (cmd === "startup_status") return { state: "opening" };
    },
    { shouldMockEvents: true },
  );
  app = mount(App, { target: document.body });
  flushSync();
  const main = document.querySelector("main.app");
  expect(main).not.toBeNull();
  await vi.waitFor(() => {
    flushSync();
    expect(main?.getAttribute("data-startup")).toBe("opening");
  });
});
