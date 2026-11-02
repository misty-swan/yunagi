// IME（日本語入力）変換中の Enter を「追加」として扱わないための判定。
// docs/design/04-interaction-spec.md 3章の規則をそのまま実装した比較用のもの。
// Svelte 版・Solid 版でまったく同じファイルを使う（scripts/sync-shared.sh でコピー）。

export type GuardReason = 'isComposing' | 'keyCode229' | 'afterCompositionEnd';

export interface ImeGuard {
  /** compositionend を受けたら呼ぶ */
  onCompositionEnd(): void;
  /** この keydown をショートカットとして扱わないなら理由を返す。扱ってよいなら null */
  ignoreReason(e: KeyboardEvent): GuardReason | null;
}

export function createImeGuard(): ImeGuard {
  // compositionend の直後に届く最初の Enter keydown を無視するための印。
  // 「同一イベントループ内の判定」（04 3章2）として、次のタスク（setTimeout 0）で必ず下ろす。
  // ミリ秒単位の時間しきい値は使わない。
  let justEnded = false;
  let clearTimer: ReturnType<typeof setTimeout> | undefined;

  return {
    onCompositionEnd() {
      justEnded = true;
      if (clearTimer !== undefined) clearTimeout(clearTimer);
      clearTimer = setTimeout(() => {
        justEnded = false;
        clearTimer = undefined;
      }, 0);
    },
    ignoreReason(e) {
      if (e.isComposing) return 'isComposing';
      if (e.keyCode === 229) return 'keyCode229';
      if (justEnded && e.key === 'Enter') {
        justEnded = false;
        return 'afterCompositionEnd';
      }
      return null;
    },
  };
}

/** 画面の「イベントの記録」に出す1行 */
export function describeEvent(e: Event, verdict?: string): string {
  const t = performance.now().toFixed(1);
  if (e instanceof KeyboardEvent) {
    return `${t} ${e.type} key=${JSON.stringify(e.key)} keyCode=${e.keyCode} isComposing=${e.isComposing}${verdict ? ' → ' + verdict : ''}`;
  }
  if (e instanceof CompositionEvent) {
    return `${t} ${e.type} data=${JSON.stringify(e.data)}${verdict ? ' → ' + verdict : ''}`;
  }
  return `${t} ${e.type}${verdict ? ' → ' + verdict : ''}`;
}

/** ベンチマーク中は記録欄の更新を止める（計測値に記録欄の描画を混ぜないため） */
export function loggingEnabled(): boolean {
  return !(window as unknown as { __yun2NoLog?: boolean }).__yun2NoLog;
}
