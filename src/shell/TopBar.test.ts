import { flushSync, mount, unmount } from "svelte";
import { afterEach, expect, test, vi } from "vitest";
import { SaveStatus } from "../lib/autosave.svelte";
import { prefs } from "../lib/prefs.svelte";
import TopBar from "./TopBar.svelte";

let bar: ReturnType<typeof mount> | undefined;

afterEach(() => {
  if (bar) unmount(bar);
  bar = undefined;
  document.body.innerHTML = "";
  prefs.lang = "ja";
});

function render(save: SaveStatus) {
  prefs.lang = "ja";
  const noop = () => {};
  bar = mount(TopBar, {
    target: document.body,
    props: {
      date: "2026-11-03",
      today: "2026-11-03",
      mode: "board",
      showLogo: false,
      unseen: 0,
      save,
      mac: true,
      onPrev: noop,
      onNext: noop,
      onToday: noop,
      onMode: noop,
      onCommand: noop,
    },
  });
  flushSync();
}

const $ = (sel: string) => document.querySelector<HTMLElement>(sel);

test("保存中だけ小さな点を出し、保存済みでは消す", () => {
  const save = new SaveStatus();
  render(save);
  expect($(".save-dot")).toBeNull();
  save.saving = true;
  flushSync();
  expect($(".save-dot")?.getAttribute("aria-label")).toBe("保存中");
  save.saving = false;
  flushSync();
  expect($(".save-dot")).toBeNull();
  // 未確認の変更が無いときは「◆ 件数」を出さない
  expect($(".agent-ind")).toBeNull();
});

test("保存に失敗している間は「保存できていません」を出し、選ぶと原因と再試行を示す", () => {
  const save = new SaveStatus();
  const retry = vi.spyOn(save, "retryNow");
  render(save);
  save.saving = true;
  save.failing = true;
  save.error = { kind: "storage", detail: "disk I/O error (os error 5)" };
  flushSync();
  expect($(".save-dot")).toBeNull();
  const fail = $(".save-fail");
  expect(fail?.textContent?.trim()).toBe("保存できていません");
  fail?.click();
  flushSync();
  const pop = $("#save-detail");
  // 原因は種類に合わせた文で、内部のエラーの文は出さない
  expect(pop?.textContent).toContain("保存先に書き込めません。");
  expect(pop?.textContent).not.toContain("disk I/O");
  ($("#save-detail .btn") as HTMLElement).click();
  expect(retry).toHaveBeenCalledOnce();
  // Esc で閉じる
  pop?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  flushSync();
  expect($("#save-detail")).toBeNull();
  // 成功したら消す
  fail?.click();
  flushSync();
  save.failing = false;
  save.error = null;
  save.saving = false;
  flushSync();
  expect($(".save-fail")).toBeNull();
  expect($("#save-detail")).toBeNull();
});

test("macOS ではキーを ⌘ で出す", () => {
  render(new SaveStatus());
  expect($(".search kbd")?.textContent).toBe("⌘K");
});
