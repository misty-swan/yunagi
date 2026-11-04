// 設定のデータ区分の操作（src/lib/settings_data.svelte.ts）：保存できていない入力が残っているときは、
// 別のデータに切り替える選び直しと復元をしない（入力が失われるため。画面設計書 7章）。
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, test, vi } from "vitest";
import type { BackupEntry } from "./bindings";
import { DataSettings } from "./settings_data.svelte";

afterEach(() => clearMocks());

const BACKUP: BackupEntry = {
  path: "/data/backups/daily-2026-11-02-1793600000.db",
  kind: { type: "daily", date: "2026-11-02" },
  createdAt: 1793600000000,
};

function setup(unsaved: boolean) {
  const calls: string[] = [];
  mockIPC((cmd) => {
    calls.push(cmd);
    if (cmd === "check_data_dir") return null;
    return null;
  });
  const order: string[] = [];
  const reload = vi.fn(() => void order.push("reload"));
  const data = new DataSettings({
    flush: async () => void order.push("flush"),
    unsaved: () => unsaved,
    reload,
    today: () => "2026-11-03",
    pick: async () => "/mnt/other",
  });
  return { data, calls, order, reload };
}

test("保存できていない入力が無ければ、入力を送ってから切り替えて、読み込み直す", async () => {
  const { data, calls, order, reload } = setup(false);
  await data.chooseSwitch("title");
  await data.confirm();
  expect(calls).toContain("use_data_dir");
  expect(order).toEqual(["flush", "reload"]);
  expect(reload).toHaveBeenCalledOnce();

  data.askRestore(BACKUP);
  await data.confirm();
  expect(calls).toContain("restore_backup");
  expect(reload).toHaveBeenCalledTimes(2);
});

test("保存できていない入力が残っているときは、選び直しも復元も、何も変えずに断る", async () => {
  const { data, calls, order, reload } = setup(true);
  await data.chooseSwitch("title");
  await data.confirm();
  expect(order).toEqual(["flush"]);
  expect(calls).not.toContain("use_data_dir");
  expect(reload).not.toHaveBeenCalled();
  expect(data.result).toEqual({ kind: "unsaved" });
  expect(data.flow).toBeNull();
  expect(data.busy).toBeNull();

  data.askRestore(BACKUP);
  await data.confirm();
  expect(calls).not.toContain("restore_backup");
  expect(reload).not.toHaveBeenCalled();
  expect(data.result).toEqual({ kind: "unsaved" });
});

test("保存先の変更は中身が同じなので、保存できていない入力があっても行い、読み込み直さない", async () => {
  const { data, calls, reload } = setup(true);
  await data.chooseMove("title");
  await data.confirm();
  expect(calls).toContain("change_data_dir");
  expect(reload).not.toHaveBeenCalled();
});
