import { expect, test } from "vitest";
import { NARROW_PX, Shell } from "./shell.svelte";

test("広いウィンドウでは、Mod+\\ で決めた状態を保つ", () => {
  const s = new Shell();
  s.setWidth(1200);
  expect(s.sidebarOpen).toBe(true);
  s.toggleSidebar();
  expect(s.sidebarOpen).toBe(false);
  s.toggleSidebar();
  expect(s.sidebarOpen).toBe(true);
});

test("ノートモードに入ると閉じ、整理モードに戻ると元に戻す", () => {
  const s = new Shell();
  s.setWidth(1200);
  s.toggleMode();
  expect(s.mode).toBe("note");
  expect(s.sidebarOpen).toBe(false);
  // ノートモードの間の開け閉めは、整理モードに戻ると捨てる
  s.toggleSidebar();
  expect(s.sidebarOpen).toBe(true);
  s.setMode("board");
  expect(s.sidebarOpen).toBe(true);
  s.toggleSidebar();
  s.setMode("note");
  s.toggleSidebar();
  s.setMode("board");
  expect(s.sidebarOpen).toBe(false);
  // 同じモードを選んでも、ノートモードの開け閉めは捨てない
  s.setMode("note");
  s.toggleSidebar();
  s.setMode("note");
  expect(s.sidebarOpen).toBe(true);
});

test("900px 未満では閉じ、明示的に開いたときだけ出す", () => {
  const s = new Shell();
  s.setWidth(NARROW_PX - 1);
  expect(s.narrow).toBe(true);
  expect(s.sidebarOpen).toBe(false);
  s.toggleSidebar();
  expect(s.sidebarOpen).toBe(true);
  // 狭いときの開け閉めは、広いときの状態を変えない
  s.toggleSidebar();
  s.setWidth(NARROW_PX);
  expect(s.sidebarOpen).toBe(true);
  // 広くなると、狭いときに開いたことは捨てる
  s.setWidth(800);
  s.toggleSidebar();
  s.setWidth(1000);
  s.setWidth(800);
  expect(s.sidebarOpen).toBe(false);
});
