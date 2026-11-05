// 自動更新の状態と「更新して再起動」（ビルド・配布手順書 5章、画面設計書 3章「New version」）。
//
// 確認はコア（Rust）が行い、状態を `update-state` イベントで知らせる。画面は聞き始めてから現在の状態を
// 問い合わせ（その間に見つかっても取りこぼさない）、サイドバー下端・上部のバー・パレットに出す。
// 確認のしくみ（更新の自動確認が無効なら通信しない、など）はコアにあり、ここにはない。

import type { UpdateState } from "./bindings";
import { core, coreEvents, keepListening } from "./core";

export interface UpdateDeps {
  /** 入力中の内容を保存し終える（再起動の前）。 */
  flush: () => Promise<unknown>;
  /** 保存に失敗している（このときは更新を始めない）。 */
  saveFailing: () => boolean;
}

export class UpdateStore {
  /** コアの最新の状態。 */
  state = $state.raw<UpdateState>({ state: "none" });
  /** 保存できていないので更新を始めなかった（次に始めるか、状態が変わるまで出す）。 */
  refused = $state(false);
  readonly #deps: UpdateDeps;
  #stopped = false;
  #unlisten: (() => void) | null = null;

  constructor(deps: UpdateDeps) {
    this.#deps = deps;
  }

  /** 出す版（新しい版がある・ダウンロード中・失敗・再起動中）。無ければ `null`。 */
  get version(): string | null {
    const s = this.state;
    return s.state === "available" || s.state === "downloading" || s.state === "failed" ? s.version : null;
  }

  /** 更新内容（無ければ `null`）。 */
  get notes(): string | null {
    return this.state.state === "available" ? this.state.notes : null;
  }

  /** 通知を出す（サイドバー下端・上部のバー）。 */
  get visible(): boolean {
    return this.state.state !== "none";
  }

  /** 「更新して再起動」を始められる（新しい版がある、または前の更新に失敗した）。 */
  get canInstall(): boolean {
    return this.state.state === "available" || this.state.state === "failed";
  }

  /** ダウンロード中の割合（0〜100。大きさが分からなければ `null`）。ダウンロード中でなければ `undefined`。 */
  get progress(): number | null | undefined {
    return this.state.state === "downloading" ? this.state.progress : undefined;
  }

  /** イベントを聞き始め、現在の状態を問い合わせる。 */
  start() {
    void (async () => {
      const u = await keepListening(
        () => coreEvents.onUpdateState((s) => this.#apply(s)),
        () => this.#stopped,
      );
      if (u === null) return;
      this.#unlisten = u;
      try {
        const s = await core.getUpdateState();
        // イベントで先に新しい状態が届いていれば、それを使う（届いていなければ none のまま）
        if (!this.#stopped && this.state.state === "none" && typeof s?.state === "string") this.#apply(s);
      } catch {
        // 読めなくても、次のイベントで分かる
      }
    })();
  }

  stop() {
    this.#stopped = true;
    this.#unlisten?.();
    this.#unlisten = null;
  }

  #apply(s: UpdateState) {
    this.state = s;
    this.refused = false;
  }

  /**
   * 「更新して再起動」。入力中の内容を保存し終えてから始める。保存できていなければ始めない
   * （`refused`）。ダウンロードの進み具合・失敗は `state` に届く。再起動するので、返ってこないことがある。
   */
  async install() {
    if (!this.canInstall) return;
    this.refused = false;
    try {
      await this.#deps.flush();
    } catch {
      // 保存し終えられなかった（保存できていないのと同じ扱い）
      this.refused = true;
      return;
    }
    if (this.#deps.saveFailing()) {
      this.refused = true;
      return;
    }
    try {
      await core.installUpdate();
    } catch {
      // コマンドが断られても（データベースを閉じた後など）、状態はイベントで届く
    }
  }
}
