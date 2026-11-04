import { expect, test } from "vitest";
import { isMacPlatform, keyLabel, noteActionOf, shellActionOf } from "./keys";

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
  expect(keyLabel("toggle", true)).toBe("⌘↵");
  expect(keyLabel("toggle", false)).toBe("Ctrl+Enter");
  expect(keyLabel("toTodo", false)).toBe("Ctrl+Shift+T");
  expect(keyLabel("moveUp", false)).toBe("Alt+↑");
  expect(keyLabel("moveUp", true)).toBe("⌥↑");
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

test("記号を Option・AltGr と打つ配列：macOS は動き、Windows・Linux は文字の入力を妨げない", () => {
  // ドイツ語の配列：macOS は Option+5 で [
  expect(shellActionOf(key({ code: "Digit5", key: "[", metaKey: true, altKey: true }), true)).toBe("prevDay");
  // Windows の AltGr+8 は Ctrl+Alt として届く。[ の入力なので扱わない
  expect(shellActionOf(key({ code: "Digit8", key: "[", ctrlKey: true, altKey: true }), false)).toBeNull();
  expect(shellActionOf(key({ code: "Minus", key: "\\", ctrlKey: true, altKey: true }), false)).toBeNull();
  // 代わりに、US 配列の [ ] \ の位置のキー（ドイツ語の配列の ü + #）で動く
  expect(shellActionOf(key({ code: "BracketLeft", key: "ü", ctrlKey: true }), false)).toBe("prevDay");
  expect(shellActionOf(key({ code: "BracketRight", key: "+", ctrlKey: true }), false)).toBe("nextDay");
  expect(shellActionOf(key({ code: "Backslash", key: "#", ctrlKey: true }), false)).toBe("sidebar");
  expect(shellActionOf(key({ code: "KeyJ", key: "j", ctrlKey: true, altKey: true }), false)).toBeNull();
  // Shift で別の文字になる記号は当たらない（US 配列の Shift+[ は {）
  expect(shellActionOf(key({ code: "BracketLeft", key: "{", ctrlKey: true, shiftKey: true }), false)).toBeNull();
});

test("ノートの時刻付き見出しは Mod+Shift+H（キーの位置で判定し、変換中と Alt 付きは扱わない）", () => {
  expect(noteActionOf(key({ code: "KeyH", key: "H", ctrlKey: true, shiftKey: true }), false)).toBe("timeHeading");
  expect(noteActionOf(key({ code: "KeyH", key: "H", metaKey: true, shiftKey: true }), true)).toBe("timeHeading");
  // ロシア語の配列でも同じ位置のキーで動く
  expect(noteActionOf(key({ code: "KeyH", key: "Р", ctrlKey: true, shiftKey: true }), false)).toBe("timeHeading");
  expect(noteActionOf(key({ code: "KeyH", key: "h", ctrlKey: true }), false)).toBeNull();
  expect(noteActionOf(key({ code: "KeyH", key: "H", metaKey: true, shiftKey: true }), false)).toBeNull();
  expect(noteActionOf(key({ code: "KeyH", key: "H", ctrlKey: true, shiftKey: true, altKey: true }), false)).toBeNull();
  expect(noteActionOf(key({ code: "KeyH", key: "H", ctrlKey: true, shiftKey: true, isComposing: true }), false)).toBeNull();
  // 行送り（Mod+Shift+T・B）
  expect(noteActionOf(key({ code: "KeyT", key: "T", ctrlKey: true, shiftKey: true }), false)).toBe("toTodo");
  expect(noteActionOf(key({ code: "KeyB", key: "B", metaKey: true, shiftKey: true }), true)).toBe("toBlocker");
  expect(noteActionOf(key({ code: "KeyT", key: "t", ctrlKey: true }), false)).toBeNull();
  expect(noteActionOf(key({ code: "KeyX", key: "X", ctrlKey: true, shiftKey: true }), false)).toBeNull();
});
