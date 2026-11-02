import { flushSync, mount, unmount } from "svelte";
import { afterEach, expect, test } from "vitest";
import App from "./App.svelte";

let app: ReturnType<typeof mount> | undefined;

// テストが失敗しても、描いた画面を必ず片付ける
afterEach(() => {
  if (app) unmount(app);
  app = undefined;
  document.body.innerHTML = "";
});

test("空の画面を描ける", () => {
  app = mount(App, { target: document.body });
  flushSync();
  expect(document.querySelector("main.app")).not.toBeNull();
});
