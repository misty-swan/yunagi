import { flushSync, mount, unmount } from "svelte";
import { afterEach, expect, test } from "vitest";
import App from "./App.svelte";

afterEach(() => {
  document.body.innerHTML = "";
});

test("空の画面を描ける", () => {
  const app = mount(App, { target: document.body });
  flushSync();
  expect(document.querySelector("main.app")).not.toBeNull();
  unmount(app);
});
