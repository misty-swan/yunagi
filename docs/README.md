# Yunagi ドキュメント一式

Yunagi は、PC画面全体を「その日の仕事ノート1枚」として使う、エンジニア向けの日次業務整理アプリです。本フォルダは、要件定義書を前提に作成した設計・公開・運用のドキュメント群です。

- 前提文書：Yunagi 要件定義書（本フォルダの Requirements-Document.md）。本フォルダ（docs/）の文書を正本とします（2026-11-02）。
- 作成日：2026-11-02
- 状態：初版ドラフト。「（案）」「TBD」と記した箇所は未確定です。

## 構成

| フォルダ | ファイル | 内容 | 必要になる時期 |
| --- | --- | --- | --- |
| （ルート） | Requirements-Document.md | 要件定義書 | 全期間 |
| design | 01-architecture.md | アーキテクチャ設計書 | 開発前 |
| design | 02-data-model.md | データモデル設計書 | 開発前 |
| design | 03-ui-spec.md | 画面設計書（UI仕様） | 開発前 |
| design | 04-interaction-spec.md | 操作仕様書（ショートカット・IME・Vimモード） | 開発前 |
| design | 05-mcp-cli-spec.md | MCP・CLI 仕様書 | 開発前 |
| design | 06-design-guide.md | デザインガイド | 開発前 |
| release | 07-build-and-distribution.md | ビルド・配布手順書 | 公開前 |
| release | 08-test-plan.md | テスト計画書 | 公開前 |
| release | SECURITY.md | セキュリティ方針の全文（GitHub が認識する窓口の短い版は、リポジトリ直下の `SECURITY.md`） | 公開前 |
| user/ja | guide.md | 利用者ガイド（日本語） | 公開前 |
| user/en | guide.md | User Guide (English) | 公開前 |
| legal | terms-of-service.md | 利用規約（草案） | 公開前 |
| legal | privacy-policy.md | プライバシーポリシー（草案） | 公開前 |
| legal | tokushoho.md | 特定商取引法に基づく表記（草案） | 公開前 |
| legal | third-party-licenses.md | サードパーティライセンス表記の方針 | 公開前 |
| future | roadmap.md | ロードマップ | 公開後 |
| future | cloud-sync-spec.md | クラウド同期仕様書（ドラフト） | 有料機能の実装前 |
| future | external-integration-spec.md | 外部ツール連携仕様書（ドラフト） | 有料機能の実装前 |
| operations | CHANGELOG.md | 変更履歴 | 公開時から |
| operations | support-runbook.md | サポート・障害対応手順 | 公開時から |

## 公式サイト（site/）

公式サイト（https://getyunagi.com）の元は、リポジトリ直下の `site/`（docs と同じ階層）にあります。利用者ガイド（`user/`）と変更履歴（`operations/CHANGELOG.md`・`CHANGELOG.en.md`）は、ビルドでサイトに取り込むので、サイトにコピーを置きません。作り方と公開の手順（オーナーの作業を含む）は、`release/07-build-and-distribution.md` の 2章です。法務文書（`legal/`）は、確定版ができるまで、サイトに載せません。

## 試作品（prototype/）

画面の見た目は、Claude Design で作る試作品（リポジトリ直下の `prototype/`、docs と同じ階層）を参考に構築します。現在の試作品はデザイン案D「夕凪」（2026-11-02 採用）で、画面とロゴ・アプリアイコンの見本は claude.ai 上の Design キャンバスにもあります。試作品と設計書の関係は次のとおりです。

- 仕様（画面の構成要素、操作、キー割当、動き、データ）の正本は設計書（03 画面設計書、04 操作仕様書）。試作品と食い違う場合は設計書に従う。
- 見た目（余白、配色、文字の大きさ・太さ、罫線）は試作品を参考にする。試作品で見た目を変えたら、結果を 06 デザインガイドに書き戻す。
- 試作品のコードは見本として扱い、本番のアプリにそのまま流用しない。
- 詳細は 03 画面設計書 12章と 06 デザインガイド 9章を参照。

## 進め方の推奨

アーキテクチャ設計書とデータモデル設計書を最初に確定させてください。ほかの設計書の多くがこの2つを前提にしています。画面は、試作品で見た目と主要な流れを確かめてから本番の実装に入ってください。法務文書は必ず弁護士等の専門家の確認を経てから公開してください。
