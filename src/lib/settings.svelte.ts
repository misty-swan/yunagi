// 設定の読み込みと保存（画面設計書 7章 S-05、データモデル設計書 3章 setting）。
//
// - 起動のとき、外枠を出す前に読む（`load`）。保存した値は、画面を描く前に当てる（等幅のノートの書体は、
//   読み込み終えてから描くため。06 4章「ノートの等幅」）。読めなかったときは既定の値のまま続ける
//   （設定が読めないことで画面を止めない）。
// - 変えたら、画面にはすぐ当ててから（`prefs`）コアへ保存する。保存できなかったら、前の値に戻して
//   `error` に失敗を持つ。保存は順に1つずつ行う（続けて変えても、届く順が入れ替わらない）。コアは
//   誤った値を丸めず断る（画面の入力欄が範囲を守るので、ふだんは起きない）ので、返された設定は使わない。

import type { SettingsPatch } from "./bindings";
import { core, errorOf, type CallError } from "./core";
import { prefs } from "./prefs.svelte";

/** `patch` の欄を `prefs` に当てる。 */
function applyPatch(patch: SettingsPatch) {
  if (patch.dayBoundary !== undefined) prefs.dayBoundary = patch.dayBoundary;
  if (patch.meterStart !== undefined) prefs.meterStart = patch.meterStart;
  if (patch.meterEnd !== undefined) prefs.meterEnd = patch.meterEnd;
  if (patch.locale !== undefined) prefs.lang = patch.locale;
  if (patch.updateCheck !== undefined) prefs.updateCheck = patch.updateCheck;
  if (patch.theme !== undefined) prefs.theme = patch.theme;
  if (patch.fontSize !== undefined) prefs.fontSize = patch.fontSize;
  if (patch.noteFont !== undefined) prefs.noteFont = patch.noteFont;
  if (patch.layout !== undefined) prefs.layout = patch.layout;
  if (patch.carryoverReviewThreshold !== undefined) prefs.reviewThreshold = patch.carryoverReviewThreshold;
  if (patch.sidebarOpen !== undefined) prefs.sidebarOpen = patch.sidebarOpen;
}

/** `prefs` のいまの値のうち、`patch` の欄だけ（失敗したときに戻すため）。 */
function snapshot(patch: SettingsPatch): SettingsPatch {
  const keys = new Set(Object.keys(patch));
  const out: SettingsPatch = {};
  if (keys.has("dayBoundary")) out.dayBoundary = prefs.dayBoundary;
  if (keys.has("meterStart")) out.meterStart = prefs.meterStart;
  if (keys.has("meterEnd")) out.meterEnd = prefs.meterEnd;
  if (keys.has("locale")) out.locale = prefs.lang;
  if (keys.has("updateCheck")) out.updateCheck = prefs.updateCheck;
  if (keys.has("theme")) out.theme = prefs.theme;
  if (keys.has("fontSize")) out.fontSize = prefs.fontSize;
  if (keys.has("noteFont")) out.noteFont = prefs.noteFont;
  if (keys.has("layout")) out.layout = prefs.layout;
  if (keys.has("carryoverReviewThreshold")) out.carryoverReviewThreshold = prefs.reviewThreshold;
  if (keys.has("sidebarOpen")) out.sidebarOpen = prefs.sidebarOpen;
  return out;
}

export class SettingsStore {
  /** 起動のときの読み込みを終えたか（読めなかったときも終えたことになる）。 */
  loaded = $state(false);
  /** 最後の保存の失敗（次の保存を始めると消す）。 */
  error = $state<CallError | null>(null);
  #queue: Promise<void> = Promise.resolve();
  #seq = 0;
  #latest: Record<string, number> = {};

  /**
   * 設定を読んで当てる。読めなくても既定の値のまま終える。`beforeShown` は、読んだ値を当てた後、外枠を
   * 出す前（`loaded` にする前）に呼ぶ（保存したサイドバーの開閉などを、最初に描くときから当てるため）。
   */
  async load(beforeShown?: () => void) {
    try {
      const s = await core.getSettings();
      if (typeof s === "object" && s !== null) prefs.apply(s);
    } catch {
      // 既定の値で続ける
    }
    beforeShown?.();
    this.loaded = true;
  }

  /**
   * 設定を変えて保存する。変える欄だけ `patch` に入れる。画面にはすぐ当て、保存できなかったら前の
   * 値に戻す。保存し終えたら `true`、失敗したら `false`。
   */
  save(patch: SettingsPatch, opts: { silent?: boolean } = {}): Promise<boolean> {
    const before = snapshot(patch);
    applyPatch(patch);
    // 欄ごとに、最後に変えた保存の番号を持つ。失敗したときは、その後にもっと新しい保存が同じ欄を
    // 変えていなければ（番号が自分のままなら）だけ戻す（新しい値を古い値で上書きしないため）
    const mine = ++this.#seq;
    for (const key of Object.keys(patch)) this.#latest[key] = mine;
    const run = this.#queue.then(async () => {
      if (!opts.silent) this.error = null;
      try {
        await core.saveSettings(patch);
        return true;
      } catch (e) {
        const revert: Record<string, unknown> = {};
        for (const [key, value] of Object.entries(before)) if (this.#latest[key] === mine) revert[key] = value;
        applyPatch(revert as SettingsPatch);
        if (!opts.silent) this.error = errorOf(e);
        return false;
      }
    });
    this.#queue = run.then(
      () => undefined,
      () => undefined,
    );
    return run;
  }
}
