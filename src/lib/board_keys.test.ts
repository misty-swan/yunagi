import { expect, test } from "vitest";
import { boardActionOf } from "./board_keys";

const key = (k: Partial<KeyboardEvent>) =>
  ({ code: "", key: "", metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, isComposing: false, keyCode: 0, ...k }) as KeyboardEvent;

test("一覧のキーを、macOS は ⌘、Windows・Linux は Ctrl で受ける", () => {
  expect(boardActionOf(key({ key: "Enter", metaKey: true }), true)).toBe("toggle");
  expect(boardActionOf(key({ key: "Enter", ctrlKey: true }), false)).toBe("toggle");
  expect(boardActionOf(key({ key: "Enter", ctrlKey: true }), true)).toBeNull();
  expect(boardActionOf(key({ key: "Enter" }), true)).toBe("enter");
  expect(boardActionOf(key({ key: "Backspace", ctrlKey: true }), false)).toBe("delete");
  expect(boardActionOf(key({ key: "ArrowUp", altKey: true }), false)).toBe("moveUp");
  expect(boardActionOf(key({ key: "ArrowDown", altKey: true }), true)).toBe("moveDown");
  expect(boardActionOf(key({ code: "KeyZ", key: "z", metaKey: true }), true)).toBe("undo");
  expect(boardActionOf(key({ code: "KeyZ", key: "Z", metaKey: true, shiftKey: true }), true)).toBe("redo");
  expect(boardActionOf(key({ code: "KeyY", key: "y", ctrlKey: true }), false)).toBe("redo");
  expect(boardActionOf(key({ code: "KeyY", key: "y", metaKey: true }), true)).toBeNull();
  expect(boardActionOf(key({ code: "KeyR", key: "R", ctrlKey: true, shiftKey: true }), false)).toBe("template");
  expect(boardActionOf(key({ code: "Digit2", key: "2", metaKey: true }), true)).toBe("regionBlocker");
});

test("IME の変換中と、Windows・Linux の AltGr（Ctrl+Alt）はショートカットにしない", () => {
  expect(boardActionOf(key({ key: "Enter", isComposing: true }), true)).toBeNull();
  expect(boardActionOf(key({ key: "Enter", keyCode: 229 }), false)).toBeNull();
  expect(boardActionOf(key({ code: "KeyZ", key: "z", ctrlKey: true, altKey: true }), false)).toBeNull();
});
