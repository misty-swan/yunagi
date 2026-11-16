// サポーターライセンスのキーの入力・確認と、「このアプリについて」の表示（画面設計書 7章「ライセンス区分と
// 『このアプリについて』」）。見た目は src/settings/LicenseSection.svelte・AboutSection.svelte。
//
// 確認（販売サービスとの通信）はコア（Rust）が行い、画面は `verify_license` を呼んで結果を待つだけ。
// 設定を閉じても確認は続き、確認できればコアが保存する。この状態は設定の画面の外に持ち、閉じた後に届いた
// 結果も受ける：確認できていれば確認済みにし、失敗していれば捨てる（閉じた後に失敗の文を出す場所を作らない）。

import type { LicenseState, VerifiedLicense, VerifyResult } from "./bindings";
import { core } from "./core";

/** 結果の文の種類。 */
export type LicenseNotice =
  | "verified"
  | "invalid_format"
  | "rejected"
  | "unreachable"
  | "service_error"
  | "save_failed"
  | "removed"
  | "remove_failed";

/** この画面が頼んでいない確認の途中を見ているときに、状態を読み直す間隔。 */
const VERIFYING_POLL_MS = 1000;

/** 確認した日（端末の時刻で `YYYY.MM.DD`）。 */
export function verifiedDate(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}.${pad(d.getMonth() + 1)}.${pad(d.getDate())}`;
}

export class LicenseStore {
  /** コアの状態（読めるまでは `null`）。 */
  state = $state.raw<LicenseState | null>(null);
  loadFailed = $state(false);
  /** 入力欄の中身（失敗したときは残す）。 */
  input = $state("");
  notice = $state<LicenseNotice | null>(null);
  /** この画面が確認を頼んで、結果を待っている。 */
  waiting = $state(false);
  /** 「キーを消す」の確かめる文を出している。 */
  confirmingRemove = $state(false);
  /** ライセンス区分か「このアプリについて」を出している。 */
  #shown = false;
  /** 確認中を読み直す時計（1 つだけ持つ）。 */
  #poll: ReturnType<typeof setTimeout> | null = null;

  /** 確認済みなら、その印。 */
  get verified(): VerifiedLicense | null {
    return this.state?.state === "verified" ? this.state.license : null;
  }

  /** 確認中（この画面が待っている、またはコアが確認中と答えた）。 */
  get verifying(): boolean {
    return this.waiting || this.state?.state === "verifying";
  }

  /** 区分を出した：状態を読み直す（通信しない）。 */
  async show() {
    this.#shown = true;
    await this.#refresh();
  }

  /** 設定を閉じた・区分を離れた：入力と文を捨てる。待っている確認は続く。 */
  hide() {
    this.#shown = false;
    this.#stopPoll();
    this.input = "";
    this.notice = null;
    this.confirmingRemove = false;
  }

  #stopPoll() {
    if (this.#poll !== null) clearTimeout(this.#poll);
    this.#poll = null;
  }

  /** 状態を読み直す（読めなければ `loadFailed`）。 */
  async #refresh() {
    try {
      this.state = await core.getLicense();
      this.loadFailed = false;
    } catch {
      this.loadFailed = true;
    }
    // 結果を待つ Promise が無いのにコアが確認中と答えた（画面を読み込み直した後など）：終わるまで読み直す
    this.#stopPoll();
    if (this.#shown && !this.waiting && this.state?.state === "verifying") {
      this.#poll = setTimeout(() => {
        this.#poll = null;
        if (this.#shown && !this.waiting) void this.#refresh();
      }, VERIFYING_POLL_MS);
    }
  }

  /** 「確認する」。空のときと、確認中は何もしない。 */
  async verify() {
    if (this.input.trim() === "" || this.verifying) return;
    this.waiting = true;
    this.notice = null;
    let result: VerifyResult | null;
    try {
      result = await core.verifyLicense(this.input);
    } catch {
      result = null;
    }
    this.waiting = false;
    if (result?.result === "verified") {
      this.state = { state: "verified", license: result.license };
      this.input = "";
      this.notice = this.#shown ? "verified" : null;
      return;
    }
    // 区分を出していない間に終わった失敗は捨てる（閉じた・離れたときに入力も捨ててある。区分を出し直して
    // いれば、文を出す）。データベースが替わった・コマンドの失敗も、文を出さない
    if (!this.#shown || result === null || result.result === "discarded" || result.result === "busy" || result.result === "unavailable") {
      if (this.#shown) await this.#refresh();
      return;
    }
    // 待っている間に読んだ状態（確認中）は、もう古い：失敗したので未確認
    if (this.state?.state === "verifying") this.state = { state: "unverified" };
    this.notice = result.result;
  }

  askRemove() {
    this.notice = null;
    this.confirmingRemove = true;
  }

  cancelRemove() {
    this.confirmingRemove = false;
  }

  /** 確かめる文で「消す」を選んだ。通信しない。 */
  async confirmRemove() {
    this.confirmingRemove = false;
    try {
      await core.removeLicense();
    } catch {
      this.notice = "remove_failed";
      return;
    }
    // 消せた。状態を読めなければ、確認済みの表示を残さない（「読めませんでした」を出す）
    this.state = null;
    await this.#refresh();
    this.notice = "removed";
  }
}

/** 「このアプリについて」の版とサードパーティライセンスの表記。 */
export class AboutStore {
  version = $state("");
  /** 表記の中身。読んでいなければ `undefined`、この版に無ければ `null`。 */
  thirdParty = $state<string | null | undefined>(undefined);
  open = $state(false);
  readonly #getVersion: () => Promise<string>;

  constructor(getVersion: () => Promise<string>) {
    this.#getVersion = getVersion;
  }

  async load() {
    if (this.version !== "") return;
    try {
      this.version = await this.#getVersion();
    } catch {
      this.version = "";
    }
  }

  /** 表記を出す・閉じる。初めて出すときに読む。 */
  async toggle() {
    if (this.open) {
      this.open = false;
      return;
    }
    if (this.thirdParty === undefined) {
      try {
        this.thirdParty = await core.getThirdPartyLicenses();
      } catch {
        this.thirdParty = null;
      }
    }
    this.open = true;
  }
}
