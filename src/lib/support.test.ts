import { describe, expect, it } from "vitest";
import { ISSUES_URL, SITE, SUPPORT_MAIL, currentOs, installCommand } from "./support";

describe("画面が案内する公式サイト・問い合わせ先", () => {
  it("決まった本物の値で、仮の表記（< >）を含まない", () => {
    expect(SITE).toBe("https://getyunagi.com");
    expect(ISSUES_URL).toBe("https://github.com/misty-swan/yunagi/issues");
    expect(SUPPORT_MAIL).toBe("support@getyunagi.com");
    for (const v of [SITE, ISSUES_URL, SUPPORT_MAIL]) expect(v).not.toMatch(/[<>]/);
  });

  it("入れ直しのコマンドは公式サイトの導入スクリプトを使う", () => {
    expect(installCommand("mac")).toBe("curl -fsSL https://getyunagi.com/install.sh | sh");
    expect(installCommand("linux", "1.2.3")).toBe("curl -fsSL https://getyunagi.com/install.sh | YUNAGI_VERSION=1.2.3 sh");
    expect(installCommand("windows")).toBe("irm https://getyunagi.com/install.ps1 | iex");
    expect(installCommand("windows", "1.2.3")).toBe('$env:YUNAGI_VERSION="1.2.3"; irm https://getyunagi.com/install.ps1 | iex');
  });

  it("OS は userAgent から決める", () => {
    expect(currentOs({ userAgent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64)" })).toBe("windows");
    expect(currentOs({ userAgent: "Mozilla/5.0 (X11; Linux x86_64)" })).toBe("linux");
  });
});
