import { flushSync, mount, unmount } from "svelte";
import { afterEach, expect, test, vi } from "vitest";
import { SaveStatus } from "../lib/autosave.svelte";
import type { UpdateState } from "../lib/bindings";
import { prefs } from "../lib/prefs.svelte";
import { UpdateStore } from "../lib/update.svelte";
import TopBar from "./TopBar.svelte";
import UpdateNotice from "./UpdateNotice.svelte";

let view: ReturnType<typeof mount> | undefined;

afterEach(() => {
  if (view) unmount(view);
  view = undefined;
  document.body.innerHTML = "";
  prefs.lang = "ja";
});

function storeOf(state: UpdateState, deps: Partial<ConstructorParameters<typeof UpdateStore>[0]> = {}) {
  const store = new UpdateStore({ flush: async () => {}, saveFailing: () => false, ...deps });
  store.state = state;
  return store;
}

const $ = <T extends HTMLElement = HTMLElement>(sel: string) => document.querySelector<T>(sel);

const found: UpdateState = { state: "available", version: "1.2.0", notes: "<b>修正</b>\n2 行目" };

test("新しい版が無いときは何も出さない", () => {
  view = mount(UpdateNotice, { target: document.body, props: { update: storeOf({ state: "none" }), placement: "side" } });
  flushSync();
  expect($(".tag")).toBeNull();
});

test("通知を選ぶと更新内容と「更新して再起動」を出し、更新内容は文字として出す（HTML にしない）", () => {
  prefs.lang = "ja";
  view = mount(UpdateNotice, { target: document.body, props: { update: storeOf(found), placement: "side" } });
  flushSync();
  expect($(".tag")?.textContent).toContain("New version");
  expect($("[role=dialog]")).toBeNull();
  $(".tag")!.click();
  flushSync();
  expect($("[role=dialog] .head")?.textContent).toBe("新しい版 1.2.0");
  expect($(".notes")?.textContent).toBe("<b>修正</b>\n2 行目");
  expect($(".notes b")).toBeNull();
  expect($(".primary")?.textContent?.trim()).toBe("更新して再起動");
});

test("英語の画面では英語の文言を出す", () => {
  prefs.lang = "en";
  view = mount(UpdateNotice, { target: document.body, props: { update: storeOf(found), placement: "top" } });
  flushSync();
  $(".tag")!.click();
  flushSync();
  expect($(".head")?.textContent).toBe("Version 1.2.0 is available");
  expect($(".primary")?.textContent?.trim()).toBe("Update and restart");
});

test("「更新して再起動」で、保存し終えてからコアに頼む", async () => {
  const flush = vi.fn(async () => {});
  const store = storeOf(found, { flush });
  const install = vi.spyOn(store, "install");
  view = mount(UpdateNotice, { target: document.body, props: { update: store, placement: "side" } });
  flushSync();
  $(".tag")!.click();
  flushSync();
  $(".primary")!.click();
  expect(install).toHaveBeenCalledOnce();
});

test("ダウンロード中は割合を出し、始め直せない。失敗したら文を出して選び直せる", () => {
  const store = storeOf({ state: "downloading", version: "1.2.0", progress: 40 });
  view = mount(UpdateNotice, { target: document.body, props: { update: store, placement: "side" } });
  flushSync();
  $(".tag")!.click();
  flushSync();
  expect($("[role=status]")?.textContent).toBe("ダウンロードしています 40%");
  expect($<HTMLButtonElement>(".primary")?.disabled).toBe(true);
  store.state = { state: "failed", version: "1.2.0" };
  flushSync();
  expect($(".warn")?.textContent).toContain("更新できませんでした");
  expect($<HTMLButtonElement>(".primary")?.disabled).toBe(false);
});

test("上部のバーには、サイドバーを閉じているときだけ出す", () => {
  const save = new SaveStatus();
  const noop = () => {};
  const mountBar = (showLogo: boolean) =>
    mount(TopBar, {
      target: document.body,
      props: {
        update: storeOf(found),
        date: "2026-11-03",
        today: "2026-11-03",
        mode: "board",
        showLogo,
        unseen: 0,
        save,
        mac: false,
        onPrev: noop,
        onNext: noop,
        onToday: noop,
        onMode: noop,
        onCommand: noop,
      },
    });
  view = mountBar(false);
  flushSync();
  expect($(".tag")).toBeNull();
  unmount(view);
  view = mountBar(true);
  flushSync();
  expect($(".top .tag")?.textContent).toContain("New version");
});
