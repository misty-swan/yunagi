import { expect, test } from "vitest";
import { isMacPlatform, keyLabel, modEnterLabel, modShiftLabel, shellActionOf } from "./keys";

function key(init: Partial<KeyboardEvent>) {
  return {
    code: "",
    key: "",
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    isComposing: false,
    keyCode: 0,
    ...init,
  };
}

test("macOS は ⌘、Windows・Linux は Ctrl で表示する", () => {
  expect(keyLabel("palette", true)).toBe("⌘K");
  expect(keyLabel("archive", true)).toBe("⇧⌘L");
  expect(keyLabel("sidebar", false)).toBe("Ctrl+\\");
  expect(keyLabel("archive", false)).toBe("Ctrl+Shift+L");
  expect(modEnterLabel(true)).toBe("⌘↵");
  expect(modShiftLabel("T", false)).toBe("Ctrl+Shift+T");
  expect(isMacPlatform({ userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0)" })).toBe(true);
  expect(isMacPlatform({ userAgent: "Mozilla/5.0 (X11; Linux x86_64)" })).toBe(false);
});

test("Mod のキーを操作に読み替える", () => {
  expect(shellActionOf(key({ code: "KeyJ", key: "j", ctrlKey: true }), false)).toBe("toggleMode");
  expect(shellActionOf(key({ code: "KeyJ", key: "j", metaKey: true }), true)).toBe("toggleMode");
  // Mod でない修飾キーでは動かない
  expect(shellActionOf(key({ code: "KeyJ", key: "j", ctrlKey: true }), true)).toBeNull();
  expect(shellActionOf(key({ code: "KeyJ", key: "j", ctrlKey: true, altKey: true }), false)).toBeNull();
  expect(shellActionOf(key({ code: "KeyT", key: "t", ctrlKey: true }), false)).toBe("today");
  // Shift の有無も見る（Mod+Shift+T はノートの送る操作）
  expect(shellActionOf(key({ code: "KeyT", key: "T", ctrlKey: true, shiftKey: true }), false)).toBeNull();
  expect(shellActionOf(key({ code: "KeyL", key: "L", ctrlKey: true, shiftKey: true }), false)).toBe("archive");
  expect(shellActionOf(key({ code: "Comma", key: ",", ctrlKey: true }), false)).toBe("settings");
});

test("記号のキーは押した文字で決める（JIS 配列でも同じ文字のキーで動く）", () => {
  // JIS 配列の [ ] は US 配列の ] \ の位置にある
  expect(shellActionOf(key({ code: "BracketRight", key: "[", ctrlKey: true }), false)).toBe("prevDay");
  expect(shellActionOf(key({ code: "Backslash", key: "]", ctrlKey: true }), false)).toBe("nextDay");
  expect(shellActionOf(key({ code: "IntlRo", key: "\\", ctrlKey: true }), false)).toBe("sidebar");
  expect(shellActionOf(key({ code: "IntlYen", key: "¥", metaKey: true }), true)).toBe("sidebar");
});

test("IME の変換中はショートカットとして扱わない", () => {
  expect(shellActionOf(key({ code: "KeyJ", key: "j", ctrlKey: true, isComposing: true }), false)).toBeNull();
  expect(shellActionOf(key({ code: "KeyJ", key: "Process", ctrlKey: true, keyCode: 229 }), false)).toBeNull();
});
