/*
  見本データ（固定。保存はしない。再読み込みで初期状態に戻る）

  - 試作品の「今日」は 2026-10-29（木）に固定する
  - 前回の日は 2026-10-28（水）。起動時の持ち越し（S-03）は、この日の未完了 TODO と未解消の障害を対象にする
  - 持ち越し回数（carry）は 02 データモデルの carry_count と同じ意味（持ち越すたびに元の値＋1）
      水曜の carry 0/1/2/3 の TODO は、持ち越すと今日 ↻1/↻2/↻3/↻4 になる。
      carry 3 の TODO は見直しの回数（既定3）以上なので、S-03 で「分割する／やめる」が出る
  - origin：human / agent / note（ノートから送った）。unseen はエージェントによる未確認の変更がある項目
  - sends：ノートから送った行の記録（02 の note_send）。行の本文が一致する間だけ送信済みの印が付く
  - 見本は夕方時点の内容。起動時の流れを見せるため、読み込み時に S-03 を表示する
  - ふりかえり（Archive）と「最近の日」のため、10-22・10-23・10-26・10-27 も少し用意する
    （古い feature flag の TODO は 10-23 から毎日持ち越されている。10-22 はすべて完了した「凪」の日）
*/
window.YUNAGI_TODAY = '2026-10-29';

window.YUNAGI_SAMPLE = {
  '2026-10-22': {
    carryDone: true,
    todos: [
      { id: 'q1', kind: 'todo', text: 'リリースノートを書く', status: 'done', carry: 0, origin: 'human' },
      { id: 'q2', kind: 'todo', text: 'ステージングで最終確認をする', status: 'done', carry: 0, origin: 'human' },
      { id: 'q3', kind: 'todo', text: 'QA の指摘を直す', status: 'done', carry: 0, origin: 'human' },
      { id: 'q4', kind: 'todo', text: '性能テストの結果をまとめる', status: 'done', carry: 0, origin: 'human' },
    ],
    blockers: [],
    reflection: ['## 良かったこと', '- リリース前の確認が予定どおり終わった', '## 詰まったこと', '- ', '## 明日の一手', '- リリース判定の資料を見直す'],
    note: ['## 10:00 リリース準備', '- 判定は明日 13:00', '- ロールバック手順を再確認する'],
    sends: [],
  },

  '2026-10-23': {
    carryDone: true,
    todos: [
      { id: 'r1', kind: 'todo', text: '古い feature flag を削除する', status: 'open', carry: 0, origin: 'human' },
      { id: 'r2', kind: 'todo', text: 'リリースを実施する', status: 'done', carry: 0, origin: 'human' },
      { id: 'r3', kind: 'todo', text: '週次レポートを提出する', status: 'done', carry: 0, origin: 'human' },
      { id: 'r4', kind: 'todo', text: '障害報告書の下書きを作る', status: 'done', carry: 0, origin: 'human' },
      { id: 'r5', kind: 'todo', text: 'CI のキャッシュ設定を見直す', status: 'done', carry: 0, origin: 'agent', agentName: 'Claude Code' },
    ],
    blockers: [
      { id: 'rb1', kind: 'blocker', text: '本番の DB マイグレーションが長い', status: 'resolved', carry: 0, origin: 'human', memo: 'インデックス作成を分けて解消' },
    ],
    reflection: ['## 良かったこと', '- リリースが予定どおり終わった', '## 詰まったこと', '- マイグレーションに想定の2倍かかった', '## 明日の一手', '- 来週の段取りを決める'],
    note: ['## 13:00 リリース判定', '- 予定どおり実施する', '- 監視は当番が担当'],
    sends: [],
  },

  '2026-10-26': {
    carryDone: true,
    todos: [
      { id: 's1', kind: 'todo', text: '古い feature flag を削除する', status: 'open', carry: 1, origin: 'human' },
      { id: 's2', kind: 'todo', text: '監視アラートの閾値を見直す', status: 'open', carry: 0, origin: 'human' },
      { id: 's3', kind: 'todo', text: '週の段取りを決める', status: 'done', carry: 0, origin: 'human' },
      { id: 's4', kind: 'todo', text: 'オンボーディング資料を更新する', status: 'done', carry: 0, origin: 'human' },
      { id: 's5', kind: 'todo', text: '1on1 の準備をする', status: 'done', carry: 0, origin: 'human' },
      { id: 's6', kind: 'todo', text: 'PR #470 をレビューする', status: 'done', carry: 0, origin: 'human' },
      { id: 's7', kind: 'todo', text: '問い合わせに回答する', status: 'done', carry: 0, origin: 'human' },
    ],
    blockers: [],
    reflection: ['## 良かったこと', '- 週の段取りを最初に決めた', '## 詰まったこと', '- 会議が午後に3件重なった', '## 明日の一手', '- レビューを午前にまとめる'],
    note: ['## 10:00 週次定例', '- 今週は新メンバーの受け入れが中心', '- 監視アラートが多すぎるという声あり'],
    sends: [],
  },

  '2026-10-27': {
    carryDone: true,
    todos: [
      { id: 'u1', kind: 'todo', text: '古い feature flag を削除する', status: 'open', carry: 2, origin: 'human' },
      { id: 'u2', kind: 'todo', text: '監視アラートの閾値を見直す', status: 'open', carry: 1, origin: 'human' },
      { id: 'u3', kind: 'todo', text: '設計書 v2 のレビュー指摘を反映する', status: 'open', carry: 0, origin: 'human' },
      { id: 'u4', kind: 'todo', text: 'PR #475 をレビューする', status: 'done', carry: 0, origin: 'human' },
      { id: 'u5', kind: 'todo', text: 'PR #477 をレビューする', status: 'done', carry: 0, origin: 'human' },
      { id: 'u6', kind: 'todo', text: '新メンバーのアカウントを申請する', status: 'done', carry: 0, origin: 'human' },
    ],
    blockers: [
      { id: 'ub1', kind: 'blocker', text: 'レビュー待ちの PR が溜まっている', status: 'resolved', carry: 0, origin: 'human', memo: '午前にまとめて片付けた' },
    ],
    reflection: ['## 良かったこと', '- レビューを午前にまとめて片付けた', '## 詰まったこと', '- レビュー待ちが多かった', '## 明日の一手', '- 新メンバーの環境づくりを手伝う'],
    note: ['## 15:00 設計書 v2 のレビュー', '- キャッシュの扱いに指摘が3件', '- 次回までに修正版を出す'],
    sends: [],
  },

  '2026-10-28': {
    carryDone: true,
    todos: [
      { id: 'p1', kind: 'todo', text: '決済 API のタイムアウトを調査する', status: 'open', carry: 0, origin: 'human' },
      { id: 'p2', kind: 'todo', text: '設計書 v2 のレビュー指摘を反映する', status: 'open', carry: 1, origin: 'human' },
      { id: 'p3', kind: 'todo', text: '監視アラートの閾値を見直す', status: 'open', carry: 2, origin: 'human' },
      { id: 'p4', kind: 'todo', text: '古い feature flag を削除する', status: 'open', carry: 3, origin: 'human' },
      { id: 'p5', kind: 'todo', text: '週次レポートを提出する', status: 'done', carry: 0, origin: 'human' },
      { id: 'p6', kind: 'todo', text: '新メンバーの開発環境づくりを手伝う', status: 'done', carry: 0, origin: 'human' },
    ],
    blockers: [
      { id: 'pb1', kind: 'blocker', text: 'ステージング DB の書き込み権限が足りない', status: 'open', carry: 0, origin: 'human' },
      { id: 'pb2', kind: 'blocker', text: 'ビルドキャッシュが効かない', status: 'resolved', carry: 0, origin: 'human', memo: 'キャッシュキーの指定ミスを直して解消' },
    ],
    reflection: [
      '## 良かったこと',
      '- 新メンバーの環境づくりが午前中に終わった',
      '## 詰まったこと',
      '- DB 権限の申請が止まっている',
      '## 明日の一手',
      '- 朝いちばんにタイムアウトのログを見る',
    ],
    note: [
      '## 11:00 チーム定例',
      '- 来週のリリース判定は木曜',
      '- 権限申請は情シスにもう一度依頼する',
      '',
      '## 15:00 新メンバー向け説明',
      '- リポジトリ構成とブランチ運用を説明した',
      '- 次回は CI の仕組み',
    ],
    sends: [],
  },

  '2026-10-29': {
    carryDone: false,
    todos: [
      { id: 't1', kind: 'todo', text: '議事メモを今日中に共有する', status: 'done', carry: 0, origin: 'note' },
      { id: 't2', kind: 'todo', text: 'PR #482 をレビューする', status: 'done', carry: 0, origin: 'human' },
      { id: 't3', kind: 'todo', text: '設計レビューの資料を仕上げる', status: 'done', carry: 0, origin: 'human' },
      { id: 't4', kind: 'todo', text: '依存パッケージ更新の PR を確認する', status: 'open', carry: 0, origin: 'agent', agentName: 'Claude Code', unseen: true },
      { id: 't5', kind: 'todo', text: 'リリースノートの下書きを作る', status: 'open', carry: 0, origin: 'note' },
    ],
    blockers: [
      { id: 'b1', kind: 'blocker', text: 'CI の e2e テストがときどき失敗する', status: 'open', carry: 0, origin: 'agent', agentName: 'Claude Code', unseen: true },
      { id: 'b2', kind: 'blocker', text: 'VPN 経由で社内 Git に接続できない', status: 'resolved', carry: 0, origin: 'human', memo: 'プロキシ設定を更新して解消' },
      { id: 'b3', kind: 'blocker', text: 'ベンダーの API 仕様書がまだ届かない', status: 'open', carry: 0, origin: 'note' },
    ],
    reflection: [
      '## 良かったこと',
      '- 設計レビューでキャッシュの方針が決まった',
      '## 詰まったこと',
      '- e2e テストの失敗が手元で再現しない',
      '## 明日の一手',
      '- ',
    ],
    note: [
      '- 朝：決済 API のタイムアウトは夜間バッチと重なっている可能性',
      '',
      '## 10:00 定例',
      '- 先週のリリースは問題なし',
      '- ベンダーの API 仕様書がまだ届かない',
      '- 議事メモを今日中に共有する',
      '- 来週から当番表が変わる',
      '',
      '## 14:00 設計レビュー',
      '- キャッシュ層は Redis で進める方針で合意',
      '- 障害時のフォールバック案は次回までに持ち寄る',
      '- リリースノートの下書きを作る',
      '',
      '## 16:30 1on1',
      '- 評価面談の日程を来週中に決める',
    ],
    sends: [
      { kind: 'blocker', text: '- ベンダーの API 仕様書がまだ届かない', itemId: 'b3' },
      { kind: 'todo', text: '- 議事メモを今日中に共有する', itemId: 't1' },
      { kind: 'todo', text: '- リリースノートの下書きを作る', itemId: 't5' },
    ],
  },
};
