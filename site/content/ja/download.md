# ダウンロードと導入

Yunagi は、ターミナルから 1 行で入れます。macOS・Windows・Linux に対応しています。管理者の権限は要りません（ログインしている利用者だけが使う形で入ります）。対応する OS は[動作環境](/ja/requirements/)にあります。

## 1. ターミナルから入れる

### macOS・Linux

```sh
curl -fsSL https://getyunagi.com/install.sh | sh
```

### Windows（PowerShell）

```powershell
irm https://getyunagi.com/install.ps1 | iex
```

スクリプトは、OS とアーキテクチャを判定し、配布物と SHA-256 のハッシュを取得して照合します。一致しなければ、何も置かずに止まります。置いた場所は、最後に一覧で表示されます（入る場所は[利用者ガイド 1 章](/ja/guide/#1-インストールと更新)にもあります）。

## 2. スクリプトの中身を読んでから実行する

`curl ... | sh` は、取得したスクリプトを読まずにそのまま実行します。中身を確かめてから実行したいときは、いったん保存して読み、そのファイルを実行してください。

macOS・Linux：

```sh
curl -fsSLo install.sh https://getyunagi.com/install.sh
less install.sh
sh install.sh
```

Windows（PowerShell）：

```powershell
irm https://getyunagi.com/install.ps1 -OutFile install.ps1
notepad install.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File .\install.ps1
```

スクリプトは、このサイトでも読めます（[install.sh](/install.sh)、[install.ps1](/install.ps1)）。正本は公開リポジトリの [`install/install.sh`](https://github.com/misty-swan/yunagi/blob/main/install/install.sh) と [`install/install.ps1`](https://github.com/misty-swan/yunagi/blob/main/install/install.ps1) で、サイトには同じ内容を置いています。スクリプトの内容は署名では守られていません。気になるときは、リポジトリの内容と見比べてください。

保存して実行すると、失敗したときの終了コードも受け取れるので、自動化にも向いています（`irm ... | iex` は終了コードを返しません）。

## 3. 版を指定して入れる

環境変数 `YUNAGI_VERSION` に版を指定すると、その版を入れます。旧版へ戻すときに使います。

```sh
curl -fsSL https://getyunagi.com/install.sh | YUNAGI_VERSION=1.2.3 sh
```

```powershell
$env:YUNAGI_VERSION="1.2.3"; irm https://getyunagi.com/install.ps1 | iex
```

指定しないときは、このサイトの `latest.json` にある最新版を入れます。

## 4. 手動でダウンロードする

<!-- slot:files -->

手動で取得したファイルは、使う前に[配布物の確かめ方](/ja/verify/)で照合してください。

- macOS：`.app.tar.gz` を展開し、`Yunagi.app` を好きな場所（たとえば `~/Applications`）に置きます。ブラウザで取ったときは、初回の起動が macOS の警告で止められることがあります。そのときは、ダウンロード時に付いた印（隔離属性）を `xattr -dr com.apple.quarantine ~/Applications/Yunagi.app` で外してから起動してください（置いた場所に合わせて、パスを読み替えます）。
- Linux：AppImage に実行の権限を付け（`chmod +x`）、実行します。起動には FUSE 2 が要ります。
- Windows：`-setup.exe` を実行します。

コマンド `yunagi` のリンクや `PATH` の設定は、インストールスクリプトが行います。手動で入れた場合は、自分で設定してください。

## 5. アンインストール

`yunagi uninstall` で、アプリと `yunagi` のリンクを消せます。データは、既定では残ります。詳しくは[利用者ガイド 8 章](/ja/guide/#8-アンインストール)にあります。
