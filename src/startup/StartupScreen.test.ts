// 起動時の画面（画面設計書 10.1）：アプリ全体を描き、コアの状態ごとの案内と操作を確かめる。
import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import App from "../App.svelte";
import { EVENTS, type BackupEntry, type Startup } from "../lib/bindings";
import { prefs } from "../lib/prefs.svelte";

let app: ReturnType<typeof mount> | undefined;

beforeEach(() => {
  prefs.lang = "ja";
});

afterEach(async () => {
  if (app) unmount(app);
  app = undefined;
  document.body.innerHTML = "";
  await new Promise((resolve) => setTimeout(resolve, 0));
  clearMocks();
});

const TODAY = "2026-11-03";

const BACKUPS: BackupEntry[] = [
  {
    path: "/data/backups/daily-2026-11-02-1793600000.db",
    kind: { type: "daily", date: "2026-11-02" },
    createdAt: 1793600000000,
  },
  {
    path: "/data/backups/pre-migration-v2-to-v3-1793500000.db",
    kind: { type: "pre_migration", from: 2, to: 3 },
    createdAt: 1793500000000,
  },
];

const NOT_FOUND: Startup = {
  state: "not_found",
  path: "/data/yunagi.db",
  dataDir: "/data",
  configuredIn: "/home/u/.local/share/yunagi/config.json",
  dataDirExists: true,
};

function failed(reason: Extract<Startup, { state: "failed" }>["reason"], more: object = {}): Startup {
  return {
    state: "failed",
    reason,
    detail: `log text for ${reason}`,
    database: "/data/yunagi.db",
    backup: null,
    config: null,
    ...more,
  };
}

type Handler = (cmd: string, args: Record<string, unknown>) => unknown;

/** 模擬のコア。`startup` で始まり、`handle` が先に答える（`undefined` を返せば既定の答え）。 */
function mockCore(startup: Startup, handle: Handler = () => undefined) {
  const calls: { cmd: string; args: Record<string, unknown> }[] = [];
  mockIPC(
    (cmd, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      calls.push({ cmd, args: a });
      const answer = handle(cmd, a);
      if (answer !== undefined) return answer;
      switch (cmd) {
        case "plugin:app|version":
          return "0.4.0";
        case "startup_status":
          return startup;
        case "list_backups":
          return BACKUPS;
        case "today":
          return TODAY;
        case "get_board":
          return { date: a.date, todos: [], blockers: [], reflection: "", note: { headings: [], length: 0 }, reflectionRev: 0, noteRev: 0 };
        case "get_text_doc":
          return { body: "", rev: 0 };
        case "report_ui_state":
        case "carryover_status":
          return null;
        case "get_overview":
          return {
            counts: { todos: 0, done: 0, leftTodos: 0, leftBlockers: 0, resolved: 0, carried: 0 },
            recent: [],
            unseenChanges: 0,
          };
      }
    },
    { shouldMockEvents: true },
  );
  return calls;
}

async function until(check: () => boolean) {
  await vi.waitFor(() => {
    flushSync();
    expect(check()).toBe(true);
  });
}

const $ = (sel: string) => document.querySelector<HTMLElement>(sel);
const text = (sel: string) => $(sel)?.textContent?.replace(/\s+/g, " ").trim() ?? "";
const buttons = () => [...document.querySelectorAll<HTMLButtonElement>(".startup button")];
const button = (label: string) => {
  const b = buttons().find((b) => b.textContent?.trim() === label);
  if (!b) throw new Error(`no button ${label}: ${buttons().map((b) => b.textContent?.trim()).join(", ")}`);
  return b;
};
const click = (label: string) => {
  button(label).click();
  flushSync();
};

async function start(startup: Startup, handle?: Handler) {
  const calls = mockCore(startup, handle);
  app = mount(App, { target: document.body });
  await until(() => $(".startup")?.dataset.startup === startup.state);
  return calls;
}

test("開いている途中は、外枠を出さずに静かに待ち、長くなったらロックを待っていることを添える", async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  try {
    await start({ state: "opening" });
    expect(text(".startup h1")).toBe("データを開いています");
    expect($(".top")).toBeNull();
    expect(text(".startup")).not.toContain("最大 5 秒");
    vi.advanceTimersByTime(1100);
    flushSync();
    expect(text(".startup")).toContain("CLI・MCP がデータを使っているときは、終わるまで最大 5 秒待ちます。");
  } finally {
    vi.useRealTimers();
  }
});

test("見つからないときは、探した場所と4つの案内を出す", async () => {
  await start(NOT_FOUND);
  expect(text(".startup h1")).toBe("データベースが見つかりません");
  expect(text(".startup")).toContain("/data/yunagi.db");
  expect(text(".startup")).toContain("保存先は /home/u/.local/share/yunagi/config.json に記録されています。");
  const options = [...document.querySelectorAll(".startup .option h2")].map((h) => h.textContent);
  expect(options).toEqual(["待ってからやり直す", "保存先を選び直す", "バックアップから戻す", "新しく作る"]);
  // バックアップは新しいものが選ばれている
  await until(() => document.querySelectorAll(".backup").length === 2);
  const labels = [...document.querySelectorAll(".backup .kind")].map((e) => e.textContent);
  expect(labels).toEqual(["定期 11.02 Mon", "更新前（スキーマ 2 → 3）"]);
  expect(document.querySelector<HTMLInputElement>(".backup input")?.checked).toBe(true);
});

test("フォルダが無いときは、ドライブ・同期の案内を出す", async () => {
  await start({ ...NOT_FOUND, dataDirExists: false, configuredIn: null });
  expect(text(".startup")).toContain("保存先のフォルダがありません。");
  expect(text(".startup")).not.toContain("に記録されています");
});

test("やり直して同じ状態に戻っても、バックアップの一覧を読み直す", async () => {
  let lists = 0;
  await start(NOT_FOUND, (cmd) => {
    if (cmd === "retry_open") return NOT_FOUND;
    if (cmd === "list_backups") return lists++ === 0 ? [] : BACKUPS;
    return undefined;
  });
  await until(() => text(".startup").includes("使えるバックアップがありません。"));
  click("やり直す");
  await until(() => document.querySelectorAll(".backup").length === 2);
});

test("やり直すと retry_open を呼び、届いた状態に変わる", async () => {
  const calls = await start(NOT_FOUND, (cmd) => (cmd === "retry_open" ? NOT_FOUND : undefined));
  click("やり直す");
  await until(() => calls.some((c) => c.cmd === "retry_open"));
  await emit(EVENTS.startup, { state: "ready" });
  await until(() => $(".top") !== null);
  expect($(".startup")).toBeNull();
});

test("保存先を選び直す：使えないフォルダは理由を出し、入れたパスで choose_data_dir を呼ぶ", async () => {
  let answer: unknown = { kind: "rejected", reason: "no_database" };
  const calls = await start(NOT_FOUND, (cmd) => {
    if (cmd !== "choose_data_dir") return undefined;
    if (answer instanceof Object && "kind" in answer) throw answer;
    return answer;
  });
  const input = $(".choose input") as HTMLInputElement;
  input.value = "  /mnt/drive/yunagi  ";
  input.dispatchEvent(new Event("input"));
  flushSync();
  click("このフォルダを使う");
  await until(() => text(".startup .error") === "そのフォルダに yunagi.db がありません。");
  expect(calls.find((c) => c.cmd === "choose_data_dir")?.args).toEqual({ dir: "/mnt/drive/yunagi" });

  answer = { state: "ready" };
  click("このフォルダを使う");
  await until(() => calls.filter((c) => c.cmd === "choose_data_dir").length === 2);
  await until(() => $(".startup .error") === null);
});

test("新しく作るは、確かめてから create_database を呼ぶ", async () => {
  const calls = await start(NOT_FOUND, (cmd) => (cmd === "create_database" ? { state: "ready" } : undefined));
  click("新しく作る");
  expect(text(".startup")).toContain("空のデータで始めます。よろしいですか？");
  click("やめる");
  expect(calls.some((c) => c.cmd === "create_database")).toBe(false);
  click("新しく作る");
  click("作る");
  await until(() => calls.some((c) => c.cmd === "create_database"));
});

test("バックアップから戻すと、退避した場所を示し、続けると整理モードへ進む", async () => {
  const restored: Startup = { state: "restored", setAside: "/data/backups/damaged-1793700000.db" };
  const calls = await start(failed("corrupted"), (cmd) => {
    if (cmd === "restore_at_startup") {
      // コアは、結果を返す前に startup を出す
      return (async () => {
        await emit(EVENTS.startup, restored);
        return restored;
      })();
    }
    if (cmd === "acknowledge_restore") {
      return (async () => {
        await emit(EVENTS.startup, { state: "ready" });
        return { state: "ready" };
      })();
    }
    return undefined;
  });
  expect(text(".startup h1")).toBe("データベースが壊れているため、開けませんでした");
  expect(text(".startup")).toContain("データベースには触れていません。");
  await until(() => document.querySelectorAll(".backup").length === 2);
  // 古い方を選んで戻す
  document.querySelectorAll<HTMLInputElement>(".backup input")[1].click();
  flushSync();
  click("選んだバックアップから戻す");
  await until(() => text(".startup h1") === "バックアップから戻しました");
  expect(calls.find((c) => c.cmd === "restore_at_startup")?.args).toEqual({ backup: BACKUPS[1].path });
  expect(text(".startup")).toContain("それまでのデータベースは /data/backups/damaged-1793700000.db に残しています。");
  expect($(".top")).toBeNull();
  // 戻すコマンドが終わるまで（知らせが先に届く）は押せない
  await until(() => !button("続ける").disabled);
  click("続ける");
  await until(() => $(".top") !== null);
  expect(calls.some((c) => c.cmd === "acknowledge_restore")).toBe(true);
});

test("戻した後に読み込み直しても、確かめるまで退避した場所を出す", async () => {
  await start({ state: "restored", setAside: "/data/backups/damaged-7.db" });
  expect(text(".startup h1")).toBe("バックアップから戻しました");
  expect(text(".startup")).toContain("/data/backups/damaged-7.db");
  expect($(".top")).toBeNull();
});

test("使えないバックアップを選んだときは、理由を出してそのままにする。読み直した一覧が空でも理由は消さない", async () => {
  let lists = 0;
  await start(failed("corrupted"), (cmd) => {
    if (cmd === "restore_at_startup") throw { kind: "rejected", reason: "corrupted" };
    if (cmd === "list_backups") return lists++ === 0 ? BACKUPS : [];
    return undefined;
  });
  await until(() => document.querySelectorAll(".backup").length === 2);
  click("選んだバックアップから戻す");
  await until(() => text(".startup .error") === "壊れているため使えません。");
  await until(() => text(".startup").includes("使えるバックアップがありません。"));
  expect(text(".startup .error")).toBe("壊れているため使えません。");
  expect(text(".startup h1")).toBe("データベースが壊れているため、開けませんでした");
});

test("マイグレーションの失敗：データが無事なこと、前の版の入れ直し方、問い合わせ先、詳しい情報を出す", async () => {
  await start(failed("migration_failed", { backup: "/data/backups/pre-migration-v2-to-v3-1.db" }));
  expect(text(".startup h1")).toBe("更新後の準備に失敗しました");
  const all = text(".startup");
  expect(all).toContain("データは無事です。");
  expect(all).toContain("/data/backups/pre-migration-v2-to-v3-1.db");
  expect(all).toContain("今の版は v0.4.0");
  expect(text(".startup .command")).toMatch(/^curl -fsSL https:\/\/.+\/install\.sh \| YUNAGI_VERSION=<版> sh$/);
  expect(all).toContain("問い合わせ先");
  expect(all).toContain("GitHub Issues");
  expect(text(".startup .detail pre")).toBe("v0.4.0 log text for migration_failed");
  // やり直しても同じなので、やり直しは出さない
  expect(buttons().some((b) => b.textContent?.trim() === "やり直す")).toBe(false);
});

test("新しい版のデータ：データが無事なこと、新しい版の入れ方、問い合わせ先を出す", async () => {
  await start(failed("unsupported_schema"));
  expect(text(".startup h1")).toBe("このデータは新しい版の Yunagi で作られています");
  expect(text(".startup")).toContain("データは無事です。");
  expect(text(".startup .command")).toMatch(/install\.sh \| sh$/);
  expect(text(".startup")).toContain("問い合わせ先");
  expect($(".backup")).toBeNull();
});

test("英語でも同じ案内を出し、Windows では PowerShell のコマンドにする", async () => {
  prefs.lang = "en";
  const ua = vi.spyOn(navigator, "userAgent", "get").mockReturnValue("Mozilla/5.0 (Windows NT 10.0; Win64; x64)");
  try {
    await start(failed("migration_failed"));
    expect(text(".startup h1")).toBe("Preparing the data after the update failed");
    expect(text(".startup")).toContain("Your data is safe.");
    expect(text(".startup .command")).toMatch(/^\$env:YUNAGI_VERSION="<version>"; irm https:\/\/.+\/install\.ps1 \| iex$/);
  } finally {
    ua.mockRestore();
  }
});

test("ロックが取れないときは、やり直しを出す", async () => {
  const calls = await start(failed("lock_busy", { database: null }), (cmd) =>
    cmd === "retry_open" ? { state: "ready" } : undefined,
  );
  expect(text(".startup h1")).toBe("ほかの処理がデータを使っています");
  expect(text(".startup")).not.toContain("問い合わせ先");
  click("やり直す");
  await until(() => calls.some((c) => c.cmd === "retry_open"));
});

test("設定ファイルが壊れているときは、その場所を出す", async () => {
  await start(failed("config", { database: null, config: "/home/u/.local/share/yunagi/config.json" }));
  expect(text(".startup")).toContain("/home/u/.local/share/yunagi/config.json");
  expect(text(".startup")).toContain("設定ファイル");
});

test("定期バックアップを作れなかったら、上部のバーに知らせる", async () => {
  mockCore({ state: "ready" });
  app = mount(App, { target: document.body });
  await until(() => $(".top") !== null);
  await emit(EVENTS.backupFailed, { detail: "could not create directory /data/backups" });
  await until(() => text(".top .notice") === "今日のバックアップを作れませんでした。自動で作り直します");
});

test("戻せなかったときは一覧を読み直し、前のデータベースを戻せなかったときはその場所と問い合わせ先を出す", async () => {
  const calls = await start(failed("corrupted"), (cmd) => {
    if (cmd === "restore_at_startup") {
      throw { kind: "put_back_failed", setAside: "/data/backups/damaged-5.db", detail: "x" };
    }
    return undefined;
  });
  await until(() => document.querySelectorAll(".backup").length === 2);
  const lists = calls.filter((c) => c.cmd === "list_backups").length;
  click("選んだバックアップから戻す");
  await until(() => text(".startup .error").includes("/data/backups/damaged-5.db"));
  await until(() => calls.filter((c) => c.cmd === "list_backups").length > lists);
  // 問い合わせ先は1つだけ出す
  expect([...document.querySelectorAll(".startup h2")].filter((h) => h.textContent === "問い合わせ先")).toHaveLength(1);
});

test("始める前に止まったときは、見出しの下に知らせる", async () => {
  await start(NOT_FOUND, (cmd) => {
    if (cmd === "choose_data_dir") throw { kind: "not_ready" };
    return undefined;
  });
  const input = $(".choose input") as HTMLInputElement;
  input.value = "/mnt/x";
  input.dispatchEvent(new Event("input"));
  flushSync();
  click("このフォルダを使う");
  await until(() => text(".startup .error").startsWith("始める前に止まりました"));
  expect(document.querySelectorAll(".startup .error")).toHaveLength(1);
});
