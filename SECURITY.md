# セキュリティ方針

Yunagi のセキュリティの考え方と保護の仕組みの全文は [docs/release/SECURITY.md](docs/release/SECURITY.md) にあります。このファイルは、GitHub が案内する窓口として、脆弱性の報告のしかたをまとめたものです。

## 脆弱性の報告

- **公開の Issue には書かないでください。**
- 報告先は、次のどちらかです。
  - メール：`security@getyunagi.com`
  - このリポジトリの「Security」タブ →「Report a vulnerability」（GitHub の Private vulnerability reporting。非公開で届きます）
- 書いてほしいこと：影響を受ける版（`yunagi --version`）、OS、再現の手順、何が起きるか。業務ノートなどの実データは貼らないでください。
- 受領の連絡は 3 営業日以内、初期評価の結果は 10 営業日以内に返すことを目標にしています（約束ではありません）。
- 修正版を公開したあと、報告者の同意があれば、謝辞を載せます。

## サポートする版

セキュリティ修正は、最新のマイナー版（例：1.4.x）のパッチ版として出します。それより古いマイナー版には出しません。更新をご案内します。

## 告知

セキュリティ修正と鍵の失効は、公式サイト（`https://getyunagi.com`）のお知らせと、このリポジトリの GitHub Security Advisory で告知します。
