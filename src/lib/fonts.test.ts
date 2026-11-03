import { expect, test } from "vitest";
import { FontLoader, loadFontFace } from "./fonts.svelte";

function fontSet(load: (spec: string) => Promise<{ status: string }[]>): FontFaceSet {
  return { load } as unknown as FontFaceSet;
}

test("読み込みの結果を確かめる：当たるフォントが無い・読めないときは使えない", async () => {
  const specs: string[] = [];
  const ok = fontSet(async (s) => {
    specs.push(s);
    return [{ status: "loaded" }];
  });
  expect(await loadFontFace("UDEV Gothic HS", ok)).toBe(true);
  expect(specs).toEqual(['400 15px "UDEV Gothic HS"']);
  // @font-face が無いと空の一覧で解決する
  expect(await loadFontFace("X", fontSet(async () => []))).toBe(false);
  expect(await loadFontFace("X", fontSet(async () => [{ status: "error" }]))).toBe(false);
  expect(await loadFontFace("X", fontSet(() => Promise.reject(new Error("network"))))).toBe(false);
  expect(await loadFontFace("X", undefined)).toBe(false);
});

test("読み込みは1回だけ行い、読めなかったら次に読み直す", async () => {
  const results = [false, true];
  let calls = 0;
  const f = new FontLoader(async () => {
    calls++;
    return results.shift()!;
  });
  expect(f.status).toBe("idle");
  const a = f.load();
  const b = f.load();
  expect(f.status).toBe("loading");
  expect(await a).toBe(false);
  expect(await b).toBe(false);
  expect(calls).toBe(1);
  expect(f.status).toBe("failed");
  expect(await f.load()).toBe(true);
  expect(f.status).toBe("loaded");
  expect(await f.load()).toBe(true);
  expect(calls).toBe(2);
});
