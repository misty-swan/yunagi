// jsdom は型の定義を持たない（@types/jsdom は入れない。依存を増やさないため）。
// サイトの試験（site.test.ts）が使う分だけを書く。Window の型は site/tsconfig.json の lib（DOM）にある。

declare module "jsdom" {
  export class JSDOM {
    constructor(html?: string);
    readonly window: Window & typeof globalThis;
  }
}
