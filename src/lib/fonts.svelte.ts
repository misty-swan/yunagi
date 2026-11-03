// ノートの等幅の書体（同梱の UDEV Gothic HS。デザインガイド 4章「ノートの等幅」）の読み込み。
//
// - 等幅の書体は、読み込み終えてから（`document.fonts.load()` の結果を確かめてから）ノートに当てる。
// - 読み込みは1回だけ行い、結果を覚える。読み込めなかったら、次に等幅を選び直したときに読み直す。
// - 待つのは `FONT_WAIT_MS` まで。過ぎたら本文の書体で描き、後から読み込み終えたらそのとき等幅に切り替える
//   （アプリの中のファイルなのでふつうは数十ミリ秒で終わる。終わらないまま文字を隠し続けないため）。

/** 同梱の等幅の書体の名前（src/styles/base.css の @font-face と同じ）。 */
export const MONO_FAMILY = "UDEV Gothic HS";

/** 等幅の書体を読み込み終えるまで、ノートを描くのを待つ上限（ミリ秒）。 */
export const FONT_WAIT_MS = 2000;

/** フォントを読み込み、使えるようになったか（`document.fonts` の無い環境では `false`）。 */
export async function loadFontFace(family: string, fonts: FontFaceSet | undefined = globalThis.document?.fonts): Promise<boolean> {
  if (fonts === undefined) return false;
  try {
    // 指定に当たるフォントが無い（@font-face が無い）と空の一覧で解決する。読めなければ失敗する
    const faces = await fonts.load(`400 15px "${family}"`);
    return faces.some((f) => f.status === "loaded");
  } catch {
    return false;
  }
}

export type FontStatus = "idle" | "loading" | "loaded" | "failed";

/** 1つのフォントの読み込みの状態（画面全体で1つ）。 */
export class FontLoader {
  status = $state<FontStatus>("idle");
  #load: () => Promise<boolean>;
  #pending: Promise<boolean> | null = null;

  constructor(load: () => Promise<boolean>) {
    this.#load = load;
  }

  /** 読み込む（読み込み終えていればすぐ `true`、読み込み中ならその結果を待つ）。 */
  load(): Promise<boolean> {
    if (this.status === "loaded") return Promise.resolve(true);
    if (this.#pending !== null) return this.#pending;
    this.status = "loading";
    this.#pending = this.#load().then(
      (ok) => {
        this.status = ok ? "loaded" : "failed";
        this.#pending = null;
        return ok;
      },
      () => {
        this.status = "failed";
        this.#pending = null;
        return false;
      },
    );
    return this.#pending;
  }
}

/** ノートの等幅の書体。 */
export const monoFont = new FontLoader(() => loadFontFace(MONO_FAMILY));
