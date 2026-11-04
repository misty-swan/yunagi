import { expect, test } from "vitest";
import {
  ACTIONS,
  bindingFromEvent,
  canonical,
  conflictOf,
  format,
  initialBinding,
  label,
  matches,
  parseBinding,
  problemOf,
  resolveKeymap,
  withBinding,
  type KeyLike,
} from "./keymap";

function key(init: Partial<KeyLike>): KeyLike {
  return { code: "", key: "", metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...init };
}

const parse = (s: string) => parseBinding(s)!;

test("割当の文字列は、並びを問わず読み、Mod・Alt・Shift の順にそろえて書く", () => {
  expect(canonical("Shift+Mod+L")).toBe("Mod+Shift+L");
  expect(canonical("Alt+Mod+[")).toBe("Mod+Alt+[");
  expect(canonical("Mod+Enter")).toBe("Mod+Enter");
  expect(canonical("Mod+1")).toBe("Mod+1");
  for (const bad of ["", "Mod+", "Mod+l", "Mod+Mod+L", "Ctrl+L", "Mod+F5", "Mod+{", "L+Mod", "Mod+LL"]) {
    expect(canonical(bad), bad).toBeNull();
  }
});

test("表記：macOS は ⌥⇧⌘ の記号、Windows・Linux は Ctrl+Alt+Shift+", () => {
  expect(label(parse("Mod+Shift+L"), true)).toBe("⇧⌘L");
  expect(label(parse("Mod+Shift+L"), false)).toBe("Ctrl+Shift+L");
  expect(label(parse("Alt+["), false)).toBe("Alt+[");
  expect(label(parse("Alt+ArrowUp"), true)).toBe("⌥↑");
  expect(label(parse("Mod+Backspace"), true)).toBe("⌘⌫");
});

test("初期割当は操作仕様書 2章のとおり。Vim モードの Windows・Linux だけ前日・翌日が Alt+[ / Alt+]", () => {
  const initial = Object.fromEntries(ACTIONS.map((a) => [a.id, a.initial]));
  expect(initial).toEqual({
    toggleMode: "Mod+J",
    prevDay: "Mod+[",
    nextDay: "Mod+]",
    today: "Mod+T",
    palette: "Mod+K",
    settings: "Mod+,",
    sidebar: "Mod+\\",
    archive: "Mod+Shift+L",
    regionTodo: "Mod+1",
    regionBlocker: "Mod+2",
    regionReflection: "Mod+3",
    toggle: "Mod+Enter",
    moveUp: "Alt+ArrowUp",
    moveDown: "Alt+ArrowDown",
    delete: "Mod+Backspace",
    template: "Mod+Shift+R",
    toTodo: "Mod+Shift+T",
    toBlocker: "Mod+Shift+B",
    timeHeading: "Mod+Shift+H",
  });
  expect(initialBinding("prevDay", true, false)).toBe("Alt+[");
  expect(initialBinding("nextDay", true, false)).toBe("Alt+]");
  expect(initialBinding("prevDay", true, true)).toBe("Mod+[");
  expect(initialBinding("prevDay", false, false)).toBe("Mod+[");
  expect(initialBinding("today", true, false)).toBe("Mod+T");
});

test("文字のキーは位置、記号のキーは押した文字で照合する", () => {
  const mod = parse("Mod+J");
  expect(matches(key({ code: "KeyJ", key: "j", ctrlKey: true }), false, mod)).toBe(true);
  // ロシア語の配列でも位置で動く
  expect(matches(key({ code: "KeyJ", key: "о", ctrlKey: true }), false, mod)).toBe(true);
  expect(matches(key({ code: "KeyJ", key: "j", ctrlKey: true, shiftKey: true }), false, mod)).toBe(false);
  expect(matches(key({ code: "KeyJ", key: "j", metaKey: true }), false, mod)).toBe(false);
  expect(matches(key({ code: "KeyJ", key: "j", ctrlKey: true }), true, mod)).toBe(false);
  // JIS 配列の [ は US 配列の ] の位置にある
  const prev = parse("Mod+[");
  expect(matches(key({ code: "BracketRight", key: "[", ctrlKey: true }), false, prev)).toBe(true);
  // ドイツ語の配列：単独で打てない記号は US 配列の位置で受ける
  expect(matches(key({ code: "BracketLeft", key: "ü", ctrlKey: true }), false, prev)).toBe(true);
  // 位置で受けるのは Shift の無いときだけ
  expect(matches(key({ code: "BracketLeft", key: "Ü", ctrlKey: true, shiftKey: true }), false, prev)).toBe(false);
  // JIS 配列の macOS の ¥
  expect(matches(key({ code: "IntlYen", key: "¥", metaKey: true }), true, parse("Mod+\\"))).toBe(true);
});

test("AltGr（Ctrl+Alt）で打つ文字は操作にしない。Windows・Linux の Alt 付きの割当は Alt の有無を合わせる", () => {
  const prev = parse("Mod+[");
  expect(matches(key({ code: "Digit8", key: "[", ctrlKey: true, altKey: true }), false, prev)).toBe(false);
  expect(matches(key({ code: "Digit8", key: "[", ctrlKey: true, altKey: true }), false, parse("Alt+["))).toBe(false);
  expect(matches(key({ code: "KeyJ", key: "j", ctrlKey: true, getModifierState: (k) => k === "AltGraph" }), false, parse("Mod+J"))).toBe(false);
  // Alt+[ は Alt だけのとき
  const alt = parse("Alt+[");
  expect(matches(key({ code: "BracketLeft", key: "[", altKey: true }), false, alt)).toBe(true);
  expect(matches(key({ code: "BracketLeft", key: "[", ctrlKey: true }), false, alt)).toBe(false);
  expect(matches(key({ code: "BracketLeft", key: "[" }), false, alt)).toBe(false);
  // Ctrl+[（Vim の Esc）は Mod+[ にだけ当たる。Alt+[ の割当には当たらない
  expect(matches(key({ code: "BracketLeft", key: "[", altKey: true }), false, prev)).toBe(false);
  // macOS で記号を Option と打つ配列（ドイツ語の Option+5 で [）でも、押した文字が合えば動く
  expect(matches(key({ code: "Digit5", key: "[", metaKey: true, altKey: true }), true, prev)).toBe(true);
});

test("名前で持つキーは押した key と Shift・Alt を合わせる", () => {
  const up = parse("Alt+ArrowUp");
  expect(matches(key({ key: "ArrowUp", altKey: true }), false, up)).toBe(true);
  expect(matches(key({ key: "ArrowUp", altKey: true }), true, up)).toBe(true);
  expect(matches(key({ key: "ArrowUp", altKey: true, shiftKey: true }), false, up)).toBe(false);
  expect(matches(key({ key: "ArrowUp" }), false, up)).toBe(false);
  const enter = parse("Mod+Enter");
  expect(matches(key({ key: "Enter", ctrlKey: true }), false, enter)).toBe(true);
  expect(matches(key({ key: "Enter" }), false, enter)).toBe(false);
});

test("割り当てられないキー", () => {
  const p = (s: string, vim = false, mac = false) => problemOf(parse(s), vim, mac);
  expect(p("Mod+L")).toBeNull();
  expect(p("Alt+[")).toBeNull();
  expect(p("L")).toBe("no_modifier");
  expect(p("Shift+L")).toBe("no_modifier");
  expect(p("Mod+Alt+L")).toBe("alt_gr");
  expect(p("Mod+Alt+L", false, true)).toBeNull();
  expect(p("Mod+Shift+[")).toBe("shifted_symbol");
  expect(p("Mod+Alt+[", false, true)).toBe("mac_option_symbol");
  expect(p("Alt+L", false, true)).toBe("mac_option_alone");
  expect(p("Alt+L")).toBeNull();
  expect(p("Alt+ArrowUp", false, true)).toBeNull();
  for (const s of ["Mod+Z", "Mod+Shift+Z", "Mod+C", "Mod+V", "Mod+X", "Mod+A", "Mod+Shift+D", "Mod+Shift+A"]) {
    expect(p(s), s).toBe("reserved");
  }
  expect(p("Mod+Y")).toBe("reserved");
  expect(p("Mod+Y", false, true)).toBeNull();
  // Ctrl+[ は、Vim モードの Windows・Linux では Esc と重なる
  expect(p("Mod+[", true)).toBe("reserved");
  expect(p("Mod+[", true, true)).toBeNull();
  expect(p("Mod+[", false)).toBeNull();
});

test("Alt だけの名前のキーは上下の矢印だけ。位置の代わりの照合は、押したキーが別の記号を打つときは使わない", () => {
  for (const s of ["Alt+Backspace", "Alt+ArrowLeft", "Alt+Enter", "Mod+ArrowLeft", "Mod+ArrowUp", "Mod+Delete", "Mod+Space", "Mod+Tab"]) {
    expect(problemOf(parse(s), false, false), s).toBe("edit_key");
  }
  expect(problemOf(parse("Mod+Backspace"), false, false)).toBeNull();
  expect(problemOf(parse("Mod+Enter"), false, false)).toBeNull();
  // オブジェクトが元から持つ名前は、記号のキーでない
  expect(parseBinding("Mod+toString")).toBeNull();
  expect(parseBinding("Mod+constructor")).toBeNull();
  // 押した文字が変わらない配列で、そのキー自身に Option・Shift を重ねたものは、記号の割当に当たらない
  expect(matches(key({ code: "BracketLeft", key: "[", metaKey: true, altKey: true }), true, parse("Mod+["))).toBe(false);
  expect(matches(key({ code: "BracketLeft", key: "[", metaKey: true, shiftKey: true }), true, parse("Mod+["))).toBe(false);
  // 別のキーで打つ記号（ドイツ語の Option+5 で [）は動く
  expect(matches(key({ code: "Digit5", key: "[", metaKey: true, altKey: true }), true, parse("Mod+["))).toBe(true);
  // フランス語の配列：US 配列の , の位置は ; を打つ。Mod+; はその割当に、Mod+, には当たらない
  const azerty = key({ code: "Comma", key: ";", ctrlKey: true });
  expect(matches(azerty, false, parse("Mod+;"))).toBe(true);
  expect(matches(azerty, false, parse("Mod+,"))).toBe(false);
  // JIS 配列の Ctrl+@（US 配列の [ の位置）は、[ の割当として記録しない
  expect(bindingFromEvent(key({ code: "BracketLeft", key: "@", ctrlKey: true }), false)).toBeNull();
});

test("同じキーを割り当てた操作は、重なる範囲の中でだけ衝突する", () => {
  const map = resolveKeymap({}, false, false);
  // 共通は、ほかのどの範囲とも重なる
  expect(conflictOf("toTodo", parse("Mod+K"), map)).toBe("palette");
  expect(conflictOf("toggle", parse("Mod+K"), map)).toBe("palette");
  // 整理とノートは重ならない
  expect(conflictOf("toBlocker", parse("Mod+Enter"), map)).toBeNull();
  expect(conflictOf("toTodo", parse("Mod+Alt+K"), map)).toBeNull();
  // 自分自身の割当は衝突しない
  expect(conflictOf("palette", parse("Mod+K"), map)).toBeNull();
  // 整理の中
  expect(conflictOf("delete", parse("Mod+Enter"), map)).toBe("toggle");
});

test("初期割当と同じ割当は持たない。持っていれば消す", () => {
  expect(withBinding({}, "palette", parse("Mod+P"), false, false)).toEqual({ palette: "Mod+P" });
  expect(withBinding({ palette: "Mod+P", today: "Mod+Q" }, "palette", parse("Mod+K"), false, false)).toEqual({ today: "Mod+Q" });
  expect(withBinding({ palette: "Mod+P" }, "palette", null, false, false)).toEqual({});
  // Vim モードの Windows・Linux の初期割当 Alt+[ も、持たない
  expect(withBinding({ prevDay: "Mod+Q" }, "prevDay", parse("Alt+["), true, false)).toEqual({});
  // Vim モードでない間の Alt+[ は、変えた割当
  expect(withBinding({}, "prevDay", parse("Alt+["), false, false)).toEqual({ prevDay: "Alt+[" });
});

test("割当の解決：変えた割当が初期割当に勝つ。読めない・使えない割当は初期割当にする", () => {
  const map = resolveKeymap({ palette: "Mod+P", today: "Mod+J", nope: "Mod+Q", sidebar: "Ctrl+Z", archive: "Mod+Alt+B" }, false, false);
  const b = (id: Parameters<typeof conflictOf>[0]) => {
    const r = map.get(id)!;
    return r.binding === null ? null : format(r.binding);
  };
  expect(b("palette")).toBe("Mod+P");
  expect(map.get("palette")).toMatchObject({ changed: true, status: "ok" });
  // today は Mod+J。初期割当で Mod+J を使っていたモードの切替は使えなくなる
  expect(b("today")).toBe("Mod+J");
  expect(b("toggleMode")).toBeNull();
  expect(map.get("toggleMode")).toMatchObject({ status: "lost", takenBy: "today", changed: false });
  // 読めない割当・使えない割当は、初期割当（ignored で知らせる）
  expect(b("sidebar")).toBe("Mod+\\");
  expect(map.get("sidebar")).toMatchObject({ status: "ignored" });
  expect(b("archive")).toBe("Mod+Shift+L");
  expect(map.get("archive")).toMatchObject({ status: "ignored" });
  // 初期割当のまま動かす操作
  expect(b("settings")).toBe("Mod+,");
});

test("変えた割当どうしが同じキーなら、設定画面の並びの先が勝つ", () => {
  const map = resolveKeymap({ palette: "Mod+P", today: "Mod+P" }, false, false);
  // today が先（toggleMode, prevDay, nextDay, today, palette）
  expect(map.get("today")).toMatchObject({ status: "ok" });
  expect(map.get("palette")).toMatchObject({ status: "lost", takenBy: "today", binding: null });
});

test("Vim モードを入れたとき、Alt+[ を別の操作に割り当てていれば、その割当が勝ち、前日の移動は使えなくなる", () => {
  const map = resolveKeymap({ today: "Alt+[" }, true, false);
  expect(map.get("today")).toMatchObject({ status: "ok", changed: true });
  expect(map.get("prevDay")).toMatchObject({ status: "lost", takenBy: "today", binding: null });
  expect(map.get("nextDay")).toMatchObject({ status: "ok" });
  // Vim モードを切ると、前日の移動は Mod+[ に戻る
  expect(resolveKeymap({ today: "Alt+[" }, false, false).get("prevDay")).toMatchObject({ status: "ok" });
});

test("Vim モードの割当に Ctrl+[ を持っていれば、使えない割当として初期割当にする", () => {
  const map = resolveKeymap({ today: "Mod+[" }, true, false);
  expect(map.get("today")).toMatchObject({ status: "ignored" });
  expect(format(map.get("prevDay")!.binding!)).toBe("Alt+[");
  // macOS では Esc と重ならない（Ctrl+[ ではなく ⌘[）。割当は有効で、初期割当が ⌘[ の前日の移動が使えなくなる
  const mac = resolveKeymap({ today: "Mod+[" }, true, true);
  expect(mac.get("today")).toMatchObject({ status: "ok" });
  expect(mac.get("prevDay")).toMatchObject({ status: "lost", takenBy: "today" });
});

test("キーの押下から割当を作る", () => {
  const f = (init: Partial<KeyLike>, mac = false) => {
    const b = bindingFromEvent(key(init), mac);
    return b === null ? null : format(b);
  };
  expect(f({ code: "KeyP", key: "p", ctrlKey: true })).toBe("Mod+P");
  expect(f({ code: "KeyP", key: "p", metaKey: true }, true)).toBe("Mod+P");
  expect(f({ code: "KeyP", key: "p", metaKey: true })).toBeNull();
  expect(f({ code: "KeyL", key: "L", ctrlKey: true, shiftKey: true })).toBe("Mod+Shift+L");
  expect(f({ code: "Digit2", key: "@", ctrlKey: true, shiftKey: true })).toBe("Mod+Shift+2");
  expect(f({ code: "BracketRight", key: "[", altKey: true })).toBe("Alt+[");
  expect(f({ code: "BracketLeft", key: "ü", ctrlKey: true })).toBe("Mod+[");
  expect(f({ code: "IntlYen", key: "¥", metaKey: true }, true)).toBe("Mod+\\");
  expect(f({ key: "ArrowUp", altKey: true })).toBe("Alt+ArrowUp");
  expect(f({ key: " ", ctrlKey: true })).toBe("Mod+Space");
  expect(f({ key: "F5", code: "F5", ctrlKey: true })).toBeNull();
  // 記号に Shift を付けて別の記号になるキー（Shift+[ は {）は、割り当てられないキーとして断る
  expect(f({ code: "BracketLeft", key: "{", ctrlKey: true, shiftKey: true })).toBeNull();
  // ASCII でない文字を打つキーの位置で、Shift を付けた記号の割当は、記号と Shift の組み合わせとして断る
  const shifted = bindingFromEvent(key({ code: "BracketLeft", key: "Ü", ctrlKey: true, shiftKey: true }), false)!;
  expect(problemOf(shifted, false, false)).toBe("shifted_symbol");
});
