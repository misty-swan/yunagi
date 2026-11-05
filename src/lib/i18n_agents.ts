// エージェントの管理の文言：変更履歴（S-06。画面設計書 8章）と、設定のエージェント区分（画面設計書 7章）で
// 共通の権限の名前・確認の文・登録の整理。i18n.ts の `Messages.agents`。
//
// 見出しの英字ラベル（Change history / Agents など）は、日英で共通（`labels.agents`）。ここには、状態の
// 説明・操作の文・エラーを各言語で書く。内部のエラーの文は出さない（README「エラーの表示」）。

import { backupTime } from "./backups";
import type { AgentScope, ApiCause, CloseReason } from "./bindings";

/** 欄の値のうち、時刻（ISO 8601）は端末の時刻の `2026.11.03 14:05` にする。読めなければそのまま。 */
function timeValue(v: string): string {
  const ms = Date.parse(v);
  return Number.isNaN(ms) ? v : backupTime(ms);
}

/** 取り消しを始められない・取り消せなかった理由（画面に出す文の種類）。 */
export type RevertProblem =
  | "user_editing"
  | "carried_over"
  | "save_failing"
  | "not_found"
  | "dropped"
  | "not_agent_change"
  | "unsupported"
  | "failed";

export interface AgentsMessages {
  /** 変更履歴のパネルの名前（読み上げ）。 */
  label: string;
  close: string;
  /** 権限の一覧の見出し。 */
  agentsTitle: string;
  /** 権限の名前（切り替えの表示）。 */
  scopes: Record<AgentScope, string>;
  /** 権限で何ができるか（許可の確認に出す）。 */
  scopeHelp: Record<AgentScope, string>;
  /** 1 つのエージェントの権限の並びの名前（読み上げ）。 */
  scopesOf: (name: string) => string;
  /** 登録の説明（固定の 2 つ）。 */
  fixed: { cli: string; unknown: string };
  lastSeen: (when: string) => string;
  neverSeen: string;
  /** `unknown` は `read` 以外を許可できない（切り替えの案内）。 */
  unknownReadOnly: string;
  /** 許可する前の確認（足す権限で何ができるか）。 */
  grantConfirm: (name: string, scope: string, help: string) => string;
  grantYes: string;
  grantNo: string;
  /** 登録を消す。 */
  remove: string;
  removeLabel: (name: string) => string;
  removeConfirm: (name: string) => string;
  removeYes: string;
  removeNo: string;
  /** 消せない理由（履歴が残っている）。 */
  inUse: string;
  loadFailed: string;
  working: string;
  /** 操作の結果（失敗・食い違い）。 */
  notices: {
    scopesChanged: string;
    scopesFailed: string;
    unknownWrite: string;
    removeInUse: string;
    removeFailed: string;
    notFound: string;
  };
  /** ローカル API。 */
  api: {
    running: string;
    connections: (n: number) => string;
    stopped: string;
    unavailable: Record<ApiCause, string>;
    failed: string;
    note: string;
  };
  history: {
    /** 絞り込み：エージェント。 */
    filterLabel: string;
    filterAll: string;
    unseenOnly: string;
    markAll: string;
    markAllLabel: (n: number) => string;
    markSeen: string;
    seen: string;
    unseen: string;
    loadMore: string;
    loading: string;
    empty: string;
    emptyUnseen: string;
    loadFailed: string;
    /** 一覧の名前（読み上げ）。 */
    listLabel: string;
    /** 操作の名前（`todo.add` など）。知らない名前はそのまま出す。 */
    operation: (op: string) => string;
    /** 対象の表示。 */
    targetItem: (kind: "todo" | "blocker", date: string, text: string | null, deleted: boolean) => string;
    /** 対象の項目が見つからないとき。 */
    targetMissing: string;
    targetNote: (date: string) => string;
    targetReflection: (date: string) => string;
    /** 取り消しの記録の説明（人が取り消した）。 */
    revertRecord: string;
    reverted: string;
    /** 変更内容の要約。 */
    added: (text: string) => string;
    field: (name: string) => string;
    value: (name: string, v: string | null) => string;
    change: (before: string | null, after: string | null) => string;
    chars: (before: number, after: number) => string;
    unknownRecord: string;
    /** 取り消しの確認。 */
    revert: string;
    revertLabel: (op: string) => string;
    revertCancel: string;
    revertOk: string;
    revertForce: string;
    revertMark: string;
    previewing: string;
    planFields: string;
    planNow: string;
    planRestore: string;
    planChangedSince: string;
    planDelete: (text: string, date: string) => string;
    planKeepsCarried: (date: string) => string;
    planText: (doc: "note" | "reflection", date: string) => string;
    planRemoved: string;
    planAdded: string;
    planTruncated: string;
    planAlreadyRestored: string;
    planAlreadyReverted: string;
    forceWarning: (fields: string) => string;
    changedAfterPreview: string;
    problem: (p: RevertProblem, detail?: { date?: string; closeReason?: CloseReason }) => string;
    result: Record<"reverted" | "marked" | "already_reverted" | "client_gone", string>;
    markFailed: string;
    moreFailed: string;
  };
}

const ja: AgentsMessages = {
  label: "エージェントの変更履歴",
  close: "閉じる",
  agentsTitle: "エージェントと権限",
  scopes: {
    read: "読み取り",
    todo: "TODO 操作",
    blocker: "障害操作",
    append: "追記",
    note_edit: "ノート編集",
  },
  scopeHelp: {
    read: "ボード・項目・ノート・振り返りの取得と検索ができます。",
    todo: "TODO の追加・編集・完了・取り消しができます。",
    blocker: "障害の追加・編集・解消ができます。",
    append: "振り返りとノートの末尾に追記できます。",
    note_edit: "ノートの既存の行の書き換え・削除・並べ替えができます。",
  },
  scopesOf: (name) => `${name} の権限`,
  fixed: {
    cli: "yunagi コマンド（CLI）からの操作。誰が実行しても、この権限で扱います。",
    unknown: "名乗らない接続。読み取りより多くは許可できません。",
  },
  lastSeen: (when) => `最後の接続 ${when}`,
  neverSeen: "まだ接続していません",
  unknownReadOnly: "名乗らない接続には、読み取りより多くは許可できません。",
  grantConfirm: (name, scope, help) =>
    `${name} に「${scope}」を許可します。${help}変更は変更履歴から取り消せますが、許可はいつでも外せます。`,
  grantYes: "許可する",
  grantNo: "やめる",
  remove: "登録を消す",
  removeLabel: (name) => `${name} の登録を消す`,
  removeConfirm: (name) =>
    `${name} の登録を消します。変更履歴には何も残っていません。次に同じ識別子で接続すると、読み取りだけを許可した登録が作り直されます。`,
  removeYes: "消す",
  removeNo: "やめる",
  inUse: "変更履歴が残っているため消せません。",
  loadFailed: "エージェントの一覧を読めませんでした。",
  working: "処理しています",
  notices: {
    scopesChanged: "別の場所で権限が変わっていたため、変えませんでした。いまの権限を確かめてから、もう一度行ってください。",
    scopesFailed: "権限を変えられませんでした。",
    unknownWrite: "名乗らない接続には、読み取りより多くは許可できません。",
    removeInUse: "変更履歴が残っているため、消せませんでした。",
    removeFailed: "登録を消せませんでした。",
    notFound: "そのエージェントはもう登録されていません。一覧を読み直しました。",
  },
  api: {
    running: "動作中",
    connections: (n) => `処理中の接続 ${n} 件`,
    stopped: "停止中（データベースを開いていません）",
    unavailable: {
      socket_path_too_long: "待ち受けを始められませんでした。ソケットのパスが長すぎます。",
      path_occupied: "待ち受けを始められませんでした。ソケットの場所に、別のファイルがあります。",
      pipe_name_taken: "待ち受けを始められませんでした。パイプの名前を、別のプロセスが使っています。",
      other: "待ち受けを始められませんでした。",
    },
    failed: "状態を読めませんでした。",
    note: "接続は 1 回の依頼で閉じるので、接続中のクライアントの一覧は出しません。止める設定もありません。",
  },
  history: {
    filterLabel: "エージェント",
    filterAll: "すべて",
    unseenOnly: "未確認のみ",
    markAll: "表示中をすべて確認済みにする",
    markAllLabel: (n) => `表示中の未確認 ${n} 件をすべて確認済みにする`,
    markSeen: "確認済みにする",
    seen: "確認済み",
    unseen: "未確認",
    loadMore: "さらに読み込む",
    loading: "読み込んでいます",
    empty: "エージェントによる変更はまだありません。",
    emptyUnseen: "未確認の変更はありません。",
    loadFailed: "変更履歴を読めませんでした。",
    listLabel: "変更の一覧",
    operation: (op) =>
      ({
        "todo.add": "TODO を追加",
        "todo.edit": "TODO を編集",
        "todo.complete": "TODO を完了",
        "todo.reopen": "TODO を戻す",
        "blocker.add": "障害を追加",
        "blocker.edit": "障害を編集",
        "blocker.resolve": "障害を解消",
        "note.append": "ノートに追記",
        "note.edit": "ノートを編集",
        "reflection.append": "振り返りに追記",
        "agent.revert": "取り消し",
      })[op] ?? op,
    targetItem: (kind, date, text, deleted) =>
      `${kind === "todo" ? "TODO" : "障害"} ${date}${text === null ? "" : `「${text}」`}${deleted ? "（削除済み）" : ""}`,
    targetMissing: "項目（見つかりません）",
    targetNote: (date) => `ノート ${date}`,
    targetReflection: (date) => `振り返り ${date}`,
    revertRecord: "あなたが取り消しました",
    reverted: "取り消し済み",
    added: (text) => `「${text}」を追加`,
    field: (name) =>
      ({
        text: "本文",
        status: "状態",
        close_reason: "取りやめた理由",
        resolution_note: "解消メモ",
        completed_at: "完了時刻",
        sort_key: "並び順",
        body: "本文",
      })[name] ?? name,
    value: (name, v) => {
      if (v === null || v === "") return "（空）";
      if (name === "completed_at") return timeValue(v);
      if (name === "close_reason")
        return ({ dropped: "取りやめ", split: "分割", returned_external: "外部課題へ戻した" } as Record<string, string>)[v] ?? v;
      if (name === "status")
        return ({ open: "未完了", done: "完了", dropped: "取りやめ", resolved: "解消済み" } as Record<string, string>)[v] ?? v;
      return v;
    },
    change: (before, after) => `${before ?? "（空）"} → ${after ?? "（空）"}`,
    chars: (before, after) => `${before} 字 → ${after} 字`,
    unknownRecord: "読み取れない記録です。",
    revert: "取り消す",
    revertLabel: (op) => `${op}を取り消す`,
    revertCancel: "やめる",
    revertOk: "取り消す",
    revertForce: "上書きして取り消す",
    revertMark: "取り消し済みにする",
    previewing: "取り消す内容を確かめています",
    planFields: "次の欄を、記録の前の値に戻します。",
    planNow: "いま",
    planRestore: "戻す値",
    planChangedSince: "記録のあとに変更されています",
    planDelete: (text, date) => `${date} の「${text}」を削除します。`,
    planKeepsCarried: (date) => `${date} に持ち越した項目は残ります。`,
    planText: (doc, date) =>
      `${date} の${doc === "note" ? "ノート" : "振り返り"}を、記録の前の本文に戻します。`,
    planRemoved: "なくなる行",
    planAdded: "戻る行",
    planTruncated: "行数が多いため、途中までを表示しています。",
    planAlreadyRestored: "すでに記録の前の値に戻っています。値は書かず、取り消し済みの印だけを付けます。",
    planAlreadyReverted: "この変更はすでに取り消し済みです。",
    forceWarning: (fields) =>
      `記録のあとに「${fields}」が変更されています。上書きすると、その変更が失われます。`,
    changedAfterPreview: "確認したあとに内容が変わりました。もう一度、取り消す内容を確かめてください。",
    problem: (p, d) => {
      switch (p) {
        case "user_editing":
          return "いま入力中のため、取り消せませんでした。入力を終えてから、もう一度行ってください。";
        case "carried_over":
          return `この項目は ${d?.date ?? "次の日"} に持ち越し済みで、その日の終わりの状態として残すため、取り消せません。持ち越し先の項目を直してください。`;
        case "save_failing":
          return "保存できていない入力があるため、取り消せませんでした。保存できてから、もう一度行ってください。";
        case "not_found":
          return "取り消す対象が見つかりません（項目が削除されたか、記録が置き換わりました）。一覧を読み直しました。";
        case "dropped":
          return "取りやめた TODO は、状態も本文も変えられないため、取り消せません。";
        case "not_agent_change":
          return "エージェントの変更ではないため、取り消せません。";
        case "unsupported":
          return "読み取れない記録のため、取り消せません。";
        case "failed":
          return "取り消せませんでした。何も変えていません。";
      }
    },
    result: {
      reverted: "取り消しました。",
      marked: "すでに戻っていたため、取り消し済みの印だけを付けました。",
      already_reverted: "すでに取り消し済みでした。",
      client_gone: "絞り込んでいたエージェントの登録がなくなったため、絞り込みを外しました。",
    },
    markFailed: "確認済みの印を付けられませんでした。",
    moreFailed: "続きを読み込めませんでした。もう一度押してください。",
  },
};

const en: AgentsMessages = {
  label: "Agent change history",
  close: "Close",
  agentsTitle: "Agents and permissions",
  scopes: {
    read: "Read",
    todo: "TODOs",
    blocker: "Blockers",
    append: "Append",
    note_edit: "Note edit",
  },
  scopeHelp: {
    read: "It can read the board, items, notes and reflections, and search.",
    todo: "It can add, edit, complete and reopen TODOs.",
    blocker: "It can add, edit and resolve blockers.",
    append: "It can append to the end of notes and reflections.",
    note_edit: "It can rewrite, delete and reorder existing note lines.",
  },
  scopesOf: (name) => `Permissions of ${name}`,
  fixed: {
    cli: "Operations from the yunagi command (CLI). Whoever runs it, it is handled with these permissions.",
    unknown: "Connections that don't name themselves. They can't be given more than Read.",
  },
  lastSeen: (when) => `Last connected ${when}`,
  neverSeen: "Has not connected yet",
  unknownReadOnly: "Connections that don't name themselves can't be given more than Read.",
  grantConfirm: (name, scope, help) =>
    `Allow "${scope}" for ${name}? ${help} Its changes can be reverted from the change history, and you can take the permission back at any time.`,
  grantYes: "Allow",
  grantNo: "Cancel",
  remove: "Remove",
  removeLabel: (name) => `Remove ${name}`,
  removeConfirm: (name) =>
    `Remove ${name}? Nothing in the change history refers to it. If it connects with the same identifier again, it is registered again with Read only.`,
  removeYes: "Remove",
  removeNo: "Cancel",
  inUse: "Can't be removed: the change history still refers to it.",
  loadFailed: "Couldn't read the list of agents.",
  working: "Working",
  notices: {
    scopesChanged: "The permissions were changed somewhere else, so nothing was changed. Check the current permissions and try again.",
    scopesFailed: "Couldn't change the permissions.",
    unknownWrite: "Connections that don't name themselves can't be given more than Read.",
    removeInUse: "Couldn't remove it: the change history still refers to it.",
    removeFailed: "Couldn't remove the agent.",
    notFound: "That agent is no longer registered. The list was reloaded.",
  },
  api: {
    running: "Running",
    connections: (n) => `${n} connection${n === 1 ? "" : "s"} in progress`,
    stopped: "Stopped (the database isn't open)",
    unavailable: {
      socket_path_too_long: "Couldn't start listening. The socket path is too long.",
      path_occupied: "Couldn't start listening. Another file is at the socket location.",
      pipe_name_taken: "Couldn't start listening. Another process is using the pipe name.",
      other: "Couldn't start listening.",
    },
    failed: "Couldn't read the status.",
    note: "A connection closes after one request, so there is no list of connected clients. There is no setting to stop it.",
  },
  history: {
    filterLabel: "Agent",
    filterAll: "All",
    unseenOnly: "Unseen only",
    markAll: "Mark all shown as seen",
    markAllLabel: (n) => `Mark all ${n} shown unseen changes as seen`,
    markSeen: "Mark as seen",
    seen: "Seen",
    unseen: "Unseen",
    loadMore: "Load more",
    loading: "Loading",
    empty: "No changes by agents yet.",
    emptyUnseen: "No unseen changes.",
    loadFailed: "Couldn't read the change history.",
    listLabel: "Changes",
    operation: (op) =>
      ({
        "todo.add": "Added a TODO",
        "todo.edit": "Edited a TODO",
        "todo.complete": "Completed a TODO",
        "todo.reopen": "Reopened a TODO",
        "blocker.add": "Added a blocker",
        "blocker.edit": "Edited a blocker",
        "blocker.resolve": "Resolved a blocker",
        "note.append": "Appended to the note",
        "note.edit": "Edited the note",
        "reflection.append": "Appended to the reflection",
        "agent.revert": "Revert",
      })[op] ?? op,
    targetItem: (kind, date, text, deleted) =>
      `${kind === "todo" ? "TODO" : "Blocker"} ${date}${text === null ? "" : ` "${text}"`}${deleted ? " (deleted)" : ""}`,
    targetMissing: "Item (not found)",
    targetNote: (date) => `Note ${date}`,
    targetReflection: (date) => `Reflection ${date}`,
    revertRecord: "You reverted it",
    reverted: "Reverted",
    added: (text) => `Added "${text}"`,
    field: (name) =>
      ({
        text: "Text",
        status: "Status",
        close_reason: "Closing reason",
        resolution_note: "Resolution note",
        completed_at: "Completed at",
        sort_key: "Order",
        body: "Text",
      })[name] ?? name,
    value: (name, v) => {
      if (v === null || v === "") return "(empty)";
      if (name === "completed_at") return timeValue(v);
      if (name === "close_reason")
        return ({ dropped: "dropped", split: "split", returned_external: "returned to the external tool" } as Record<string, string>)[v] ?? v;
      if (name === "status")
        return ({ open: "open", done: "done", dropped: "dropped", resolved: "resolved" } as Record<string, string>)[v] ?? v;
      return v;
    },
    change: (before, after) => `${before ?? "(empty)"} → ${after ?? "(empty)"}`,
    chars: (before, after) => `${before} chars → ${after} chars`,
    unknownRecord: "This record can't be read.",
    revert: "Revert",
    revertLabel: (op) => `Revert: ${op}`,
    revertCancel: "Cancel",
    revertOk: "Revert",
    revertForce: "Overwrite and revert",
    revertMark: "Mark as reverted",
    previewing: "Checking what the revert would do",
    planFields: "These fields go back to the values before the change.",
    planNow: "Now",
    planRestore: "Restored",
    planChangedSince: "Changed after the record",
    planDelete: (text, date) => `Delete "${text}" of ${date}.`,
    planKeepsCarried: (date) => `The item carried over to ${date} stays.`,
    planText: (doc, date) =>
      `Put the ${doc === "note" ? "note" : "reflection"} of ${date} back to the text before the change.`,
    planRemoved: "Lines that go away",
    planAdded: "Lines that come back",
    planTruncated: "The lines are cut off because there are many.",
    planAlreadyRestored: "It is already back to the earlier values. No value is written; the record is only marked as reverted.",
    planAlreadyReverted: "This change is already reverted.",
    forceWarning: (fields) =>
      `"${fields}" was changed after the record. Overwriting loses that change.`,
    changedAfterPreview: "The content changed after you checked it. Check what the revert would do again.",
    problem: (p, d) => {
      switch (p) {
        case "user_editing":
          return "You are editing it right now, so it wasn't reverted. Finish editing and try again.";
        case "carried_over":
          return `This item was carried over to ${d?.date ?? "the next day"} and stays as the end-of-day state, so it can't be reverted. Fix the carried-over item instead.`;
        case "save_failing":
          return "Some input isn't saved yet, so it wasn't reverted. Try again once it is saved.";
        case "not_found":
          return "The target can't be found (the item was deleted or the record was replaced). The list was reloaded.";
        case "dropped":
          return "A dropped TODO can't change its status or text, so it can't be reverted.";
        case "not_agent_change":
          return "This is not a change by an agent, so it can't be reverted.";
        case "unsupported":
          return "This record can't be read, so it can't be reverted.";
        case "failed":
          return "Couldn't revert. Nothing was changed.";
      }
    },
    result: {
      reverted: "Reverted.",
      marked: "It was already back, so the record was only marked as reverted.",
      already_reverted: "It was already reverted.",
      client_gone: "The agent you were filtering by is no longer registered, so the filter was removed.",
    },
    markFailed: "Couldn't mark it as seen.",
    moreFailed: "Couldn't load more. Press the button again.",
  },
};

export const AGENTS_MESSAGES = { ja, en };
