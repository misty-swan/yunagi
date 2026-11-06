# Yunagi への協力のしかた

Yunagi に関心を持っていただき、ありがとうございます。このリポジトリは個人で運営しています。協力の受け方を、先に正直に書きます。

*English summary: Bug reports and feature requests are welcome as GitHub Issues (Japanese or English). Pull requests are read, but this is a one-person project, so a merge is not guaranteed and replies can be slow. Please open an Issue before working on anything larger than a small fix. Do not report security problems in public; see [SECURITY.md](SECURITY.md).*

## 不具合の報告・要望

- 不具合と要望は、GitHub の Issues に書いてください（日本語でも英語でも構いません）。テンプレートがあります。
- 不具合は、Yunagi の版（`yunagi --version`）、OS と CPU、再現の手順があると調べやすくなります。診断情報が要るときは `yunagi doctor` の出力をお願いします。業務ノートの内容は含まれませんが、貼る前に中身を見てください。
- アカウントを作りたくない場合は、`support@getyunagi.com` に送れます。
- 返事までの目標は 3 営業日ですが、約束ではありません。
- **脆弱性（セキュリティの問題）は、公開の Issue に書かないでください。** [SECURITY.md](SECURITY.md) の窓口に送ってください。

## プルリクエスト

- 受け付けますが、**取り込みは保証しません**。確認に時間がかかることがあり、方針に合わなければ、理由を書いて閉じることがあります。
- 誤字の修正や、はっきりした不具合の小さな修正は、そのまま PR を出して構いません。
- それより大きな変更（新しい機能、画面の構成や操作の流れを変えるもの、依存の追加など）は、**作り始める前に Issue で相談してください**。何を作るかは設計書（`docs/`）が正本で、画面や操作の方針は運営者が決めます。方針に合わず取り込めない変更を、無駄に作らせないためです。
- 1 つの PR は 1 つの目的に絞ってください。
- PR を出す前に、リポジトリ直下で `pnpm check` を通してください（Rust のテスト・clippy・型チェック・画面のテストがまとめて走ります。準備と使い方は [README.md](README.md)）。
- 設計書と食い違う変更は、設計書も一緒に直してください。
- 新しい依存を足すときは、固定・ライセンス・脆弱性の検査の決まり（[docs/release/SECURITY.md](docs/release/SECURITY.md) 4.4）に従います。許可していないライセンスの依存は入れられません。
- 外部からの PR では、リポジトリの設定により、検査（GitHub Actions）の実行に運営者の承認が要ります。承認が済むまで、検査は動きません。
- 生成 AI に手伝ってもらった変更でも構いません。ただし、中身は出す人が確かめ、説明できるようにしてください。

## 貢献したものの権利

あなたが提出した変更は、特に書かない限り、このプロジェクトと同じ **MIT と Apache-2.0 のデュアルライセンス**（[LICENSE-MIT](LICENSE-MIT)・[LICENSE-APACHE](LICENSE-APACHE)）で提供したものとして扱います。追加の条件は付きません（Apache-2.0 の 5 条と同じ考え方です）。別途の契約（CLA）への署名は求めません。

## 行動規範

[CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) に従ってください。
