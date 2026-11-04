import { flushSync, mount, unmount } from "svelte";
import { afterEach, expect, test, vi } from "vitest";
import { SaveStatus } from "../lib/autosave.svelte";
import { prefs } from "../lib/prefs.svelte";
import type { ShellCommand } from "./commands";
import TopBar from "./TopBar.svelte";

let bar: ReturnType<typeof mount> | undefined;

afterEach(() => {
  if (bar) unmount(bar);
  bar = undefined;
  document.body.innerHTML = "";
  prefs.lang = "ja";
});

function render(save: SaveStatus, onCommand: (c: ShellCommand) => void = () => {}) {
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
      onCommand,
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
  const pop = $(".pop");
  // 開くとフォーカスが原因の表示へ移り、ボタンはそれを指す
  expect(document.activeElement).toBe(pop);
  expect(fail?.getAttribute("aria-controls")).toBe(pop?.id);
  expect(fail?.getAttribute("aria-expanded")).toBe("true");
  // 原因は種類に合わせた文で、内部のエラーの文は出さない
  expect(pop?.textContent).toContain("保存先に書き込めません。");
  expect(pop?.textContent).not.toContain("disk I/O");
  ($(".pop .btn") as HTMLElement).click();
  expect(retry).toHaveBeenCalledOnce();
  // Esc で閉じ、フォーカスを「保存できていません」へ戻す（フォーカスのある要素に Esc を送る）
  document.activeElement?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  flushSync();
  expect($(".pop")).toBeNull();
  expect(document.activeElement).toBe(fail);
  expect(fail?.hasAttribute("aria-controls")).toBe(false);
  // 成功したら消す
  fail?.click();
  flushSync();
  save.failing = false;
  save.error = null;
  save.saving = false;
  flushSync();
  expect($(".save-fail")).toBeNull();
  expect($(".pop")).toBeNull();
});

test("保存できていない原因の表示の「保存先を変える」で、設定のデータ区分を開く", () => {
  const save = new SaveStatus();
  const commands: ShellCommand[] = [];
  render(save, (c) => commands.push(c));
  save.failing = true;
  save.error = { kind: "storage", detail: "x" };
  flushSync();
  $(".save-fail")?.click();
  flushSync();
  const change = [...document.querySelectorAll<HTMLElement>(".pop .btn")].find((b) => b.textContent?.trim() === "保存先を変える");
  expect(change).toBeDefined();
  change?.click();
  expect(commands).toEqual(["dataSettings"]);
});

test("macOS ではキーを ⌘ で出す", () => {
  render(new SaveStatus());
  expect($(".search kbd")?.textContent).toBe("⌘K");
});

test("再試行で保存できたら、フォーカスを上部のバーへ移す。外を押すと原因の表示を閉じる", () => {
  const save = new SaveStatus();
  render(save);
  save.failing = true;
  save.error = { kind: "not_ready" };
  flushSync();
  $(".save-fail")?.click();
  flushSync();
  expect($(".pop")?.textContent).toContain("データベースを開けていません。");
  ($(".pop .btn") as HTMLElement).focus();
  save.failing = false;
  save.error = null;
  flushSync();
  expect($(".pop")).toBeNull();
  expect(document.activeElement).toBe($(".top"));

  save.failing = true;
  flushSync();
  $(".save-fail")?.click();
  flushSync();
  expect($(".pop")).not.toBeNull();
  $(".search")?.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
  flushSync();
  expect($(".pop")).toBeNull();
});
