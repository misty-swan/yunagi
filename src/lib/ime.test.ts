// IME の切替の呼び出しの条件（操作仕様書 4.4）：Vimモードが有効で、窓が前面にあるときだけ呼ぶ。入力状態を
// 戻すのは、設定 `vim_ime_restore` が有効なときだけ。コアは模擬。
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { imeEnterInsert, imeLeaveInsert, imeSupport } from "./ime.svelte";
import { prefs } from "./prefs.svelte";

let calls: string[];
let focus: ReturnType<typeof vi.spyOn>;

beforeEach(() => {
  calls = [];
  focus = vi.spyOn(document, "hasFocus").mockReturnValue(true);
  prefs.vimMode = true;
  prefs.vimImeRestore = true;
  imeSupport.status = null;
});

afterEach(() => {
  focus.mockRestore();
  prefs.vimMode = false;
  prefs.vimImeRestore = true;
  clearMocks();
});

function mock(answer: Record<string, unknown> = {}) {
  mockIPC((cmd) => {
    calls.push(cmd);
    if (cmd in answer) {
      const v = answer[cmd];
      if (v instanceof Object && "kind" in v) throw v;
      return v;
    }
    return null;
  });
}

const settle = () => new Promise((r) => setTimeout(r, 0));

test("挿入モードを抜けたら英数に切り替え、入ったら戻す", async () => {
  mock();
  imeLeaveInsert();
  imeEnterInsert();
  await settle();
  expect(calls).toEqual(["ime_leave_insert", "ime_enter_insert"]);
});

test("設定 vim_ime_restore が無効なら、入ったときに戻さない（入ったことは知らせ、抜けたときの英数への切替は行う）", async () => {
  const args: unknown[] = [];
  mockIPC((cmd, a) => {
    calls.push(cmd);
    args.push(a);
    return null;
  });
  prefs.vimImeRestore = false;
  imeEnterInsert();
  prefs.vimImeRestore = true;
  imeEnterInsert();
  imeLeaveInsert();
  await settle();
  expect(calls).toEqual(["ime_enter_insert", "ime_enter_insert", "ime_leave_insert"]);
  expect(args).toEqual([{ restore: false }, { restore: true }, {}]);
});

test("Vimモードが無効のとき、窓が前面にないときは、呼ばない", async () => {
  mock();
  prefs.vimMode = false;
  imeLeaveInsert();
  imeEnterInsert();
  prefs.vimMode = true;
  focus.mockReturnValue(false);
  imeLeaveInsert();
  imeEnterInsert();
  await settle();
  expect(calls).toEqual([]);
});

test("切替の失敗は画面に出さない（呼び出しが断られても、例外にしない）", async () => {
  mock({ ime_leave_insert: { kind: "internal", detail: "x" } });
  const errors: unknown[] = [];
  const on = (e: PromiseRejectionEvent) => errors.push(e);
  window.addEventListener("unhandledrejection", on);
  imeLeaveInsert();
  await settle();
  window.removeEventListener("unhandledrejection", on);
  expect(calls).toEqual(["ime_leave_insert"]);
  expect(errors).toEqual([]);
});

test("切替ができない環境を問い合わせる。問い合わせに失敗したら、できないとは決めつけない", async () => {
  mock({ ime_status: { available: false, method: null } });
  await imeSupport.refresh();
  expect(imeSupport.unavailable).toBe(true);
  mock({ ime_status: { available: true, method: "fcitx5" } });
  await imeSupport.refresh();
  expect(imeSupport.unavailable).toBe(false);
  mock({ ime_status: { kind: "internal", detail: "x" } });
  await imeSupport.refresh();
  expect(imeSupport.status).toBeNull();
  expect(imeSupport.unavailable).toBe(false);
});
