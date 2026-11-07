# 配布物の確かめ方

Yunagi は、Apple の公証や Windows のコード署名（有料の開発者登録）を使っていません。そのため、OS が「開発元を確認できない」という警告を出すことがあります。かわりに、配布物が公式のものであることを、この手順で確かめられます。

インストールスクリプトで入れる場合、SHA-256 の照合はスクリプトが自動で行います（一致しなければ何も置きません）。この手順は、配布物を手動で取得したときと、スクリプトとは別に自分で確かめたいときのためのものです。

確かめることは2つです。

1. `SHA256SUMS`（全配布物のハッシュの一覧）が、Yunagi の配布用の鍵で署名されていること。
2. 取得したファイルのハッシュが、その一覧と同じであること。

## 1. 配布用の公開鍵

<!-- slot:key -->

配布用の鍵は、アプリの自動更新の署名鍵とは別のものです。公開鍵は、このサイトと公開リポジトリの両方に載せます。2か所を見比べると、片方が書き換えられても気づけます。鍵を差し替えるときは、[お知らせ](/ja/news/)で告知します。

## 2. 必要なものを集める

同じフォルダに、次の4つを保存します。

- 確かめたい配布物（[ダウンロード](/ja/download/)の「手動でダウンロードする」から取得）
- `SHA256SUMS`
- `SHA256SUMS.sig`
- `yunagi-release.pub`（上の公開鍵）

署名の確認には、[minisign](https://jedisct1.github.io/minisign/) を使います。minisign の入れ方は、その公式のページにあります。

## 3. SHA256SUMS の署名を確かめる

```sh
minisign -Vm SHA256SUMS -x SHA256SUMS.sig -p yunagi-release.pub
```

`Signature and comment signature verified` と出れば、`SHA256SUMS` は配布用の鍵で署名されたものです。失敗したときは、そのファイルを使わないでください。

## 4. ハッシュを照合する

署名を確かめた `SHA256SUMS` と、取得したファイルのハッシュを比べます。`1.2.3` と `arm64` は、取得したファイルの名前に置き換えてください。

Linux：

```sh
sha256sum -c --ignore-missing SHA256SUMS
```

macOS：

```sh
grep ' Yunagi_1.2.3_arm64.app.tar.gz$' SHA256SUMS | shasum -a 256 -c -
```

Windows（PowerShell）：

```powershell
(Get-FileHash .\Yunagi_1.2.3_x86_64-setup.exe -Algorithm SHA256).Hash.ToLower()
Select-String -Path .\SHA256SUMS -Pattern 'Yunagi_1.2.3_x86_64-setup.exe'
```

Linux と macOS は、`OK` と出れば一致しています。Windows は、1行目のハッシュ（64桁）が、2行目の行の先頭の64桁と同じであれば一致しています。

## 5. 確かめられなかったとき

そのファイルは使わないでください。ファイルを取得し直しても同じなら、[セキュリティ](/ja/security/)の窓口へ知らせてください。
