# 変更履歴

このファイルは [Keep a Changelog](https://keepachangelog.com/ja/1.1.0/) の形式に従い、バージョン番号は [セマンティック バージョニング](https://semver.org/lang/ja/) に従います。

## [Unreleased]

### Added
- 日付ボード（TODO、障害・詰まり、振り返り）
- ノートモードと行送り
- 未完了 TODO・未解消の障害の持ち越し
- 自動保存と自動バックアップ
- 全文検索
- Vimモード
- アプリが前面になくても、ショートカット（既定は Mod+Shift+D）で Yunagi を呼び出す（Wayland では効かないことがあります）
- コマンド `yunagi show [--note]` で Yunagi を前面に出す（動いていなければ起動します。Wayland でショートカットが効かないときに、キーボード設定から呼べます）
- CLI と MCP サーバーによる AI エージェント連携
- 選んだ TODO を、元のノートの区間と未解消の障害を添えた依頼文としてコピーする（Mod+Shift+A。CLI の `yunagi handoff export`、MCP の `export_handoff` でも取得できます）
- 日本語・英語の UI

<!--
記載ルール
- 区分：Added（追加）、Changed（変更）、Deprecated（非推奨）、Removed（削除）、Fixed（修正）、Security（セキュリティ）
- 利用者が読んで分かる言葉で書く。内部の実装の話は書かない。
- 英語版を CHANGELOG.en.md として用意し、同時に更新する（UI を日英で同時に提供するため）。
- 過去日の編集・Markdown 書き出しなど Should の機能は、MVP に含めると確定した時点で Added に加える（要確認）。
- 末尾に比較リンクを置く。初回リリースまでは省略してよい。
-->

[Unreleased]: https://github.com/misty-swan/yunagi/compare/vX.Y.Z...HEAD
