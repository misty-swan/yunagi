# ビルド・配布手順書

## 1. 方針

- 有料の開発者登録（Apple Developer Program、Windows のコード署名証明書）を使わずに配布する。
- 主な導入方法は、ターミナルからの1行インストール。macOS は Homebrew tap も提供する。
- アプリの自動更新は Tauri Updater を使い、Yunagi 独自の鍵で署名する。
- 配布物の実体は GitHub Releases に置き、利用者への案内は公式サイトに一本化する（Homebrew tap は公式サイトから案内する補助経路）。要件定義書 9章の「配布経路は公式サイトのみ」は「ストアでは販売しない」の意味である。
- 本体ソースの公開可否にかかわらず、配布・Issue・脆弱性報告用の公開リポジトリ（インストールスクリプト、CHANGELOG、SECURITY.md を置く）を用意する。本書で「GitHub」と書くときはこの公開リポジトリを指す。
- 全配布物の SHA-256 ハッシュと、その署名を公開する（2章）。

## 2. 配布物

| OS | アーキテクチャ | 配布物 | CLI の配置 |
| --- | --- | --- | --- |
| macOS | arm64、x86_64 | `Yunagi_<ver>_<arch>.app.tar.gz` | アプリに同梱した `yunagi` を `~/.local/bin` にリンク（Homebrew の場合は Cask の `binary` で Homebrew の bin にリンク） |
| Linux | x86_64、arm64 | `Yunagi_<ver>_<arch>.AppImage`、`.tar.gz`、`.deb`（任意） | `~/.local/opt/yunagi/bin/yunagi` を `~/.local/bin/yunagi` からリンク |
| Windows | x86_64、arm64 | `Yunagi_<ver>_<arch>-setup.exe`（NSIS、ユーザー単位インストール） | `%LOCALAPPDATA%\Programs\Yunagi\bin` を PATH に追加 |

- CLI は GUI とは別のコンソール用バイナリ `yunagi`（Windows は `yunagi.exe`）として同梱する。
- アプリ内自動更新の対象は、macOS の `.app.tar.gz`、Linux の AppImage、Windows の NSIS 版に限る。Linux の `.tar.gz`・`.deb` は自動更新の対象外で、再導入で更新する（利用者ガイドに明記する）。
- 配布するアーキテクチャは、テスト計画書 3章の対象環境と1対1で対応させる。リリース前に検証できなかった組み合わせは、ダウンロード案内とリリースノートで「試験提供（動作未保証）」と明記する。

各配布物に対応する `.sha256` と、Tauri Updater 用の署名ファイル（`.sig`）を同時に公開する。あわせて、全配布物のハッシュをまとめた `SHA256SUMS` と、その minisign 署名 `SHA256SUMS.sig` を公開する。`SHA256SUMS.sig` の署名には、Updater 鍵とは別の配布用鍵を使う。配布用の公開鍵は公式サイトと公開リポジトリの両方に掲載し、検証手順を導入ガイドに載せる。更新情報 `latest.json` は `https://<公式サイト>/latest.json` に置き、取り下げ時にすぐ差し替えられるようにする。

## 3. ビルド（CI）

GitHub Actions で OS ごとのマトリクスビルドを行う。

1. タグ `v<ver>` の push を契機に起動する。
2. 依存を固定したまま（lock ファイルを厳守）ビルドする。
3. 依存の既知脆弱性の検査と、許可外ライセンスの検出（third-party-licenses.md 2章）を行い、問題があれば中止する。同梱するフォントのファイルの SHA-256 がデザインガイド 4章に書いた値と一致し、ライセンス文書（UDEV Gothic HS は `LICENSE`・`LICENSE_JetBrainsMono`・`LICENSE_BIZUDGothic`、Inter は `LICENSE.txt`）がそろっていることも確かめ、違えば中止する。脆弱性の検査は次のとおり。
   - Rust：`cargo audit`（`Cargo.lock` を調べる）。脆弱性（vulnerability）の指摘が1件でもあれば中止する。メンテナンス終了・取り下げ（yanked）の警告は、ログに残すだけで中止しない。
   - JavaScript：`pnpm audit`（`pnpm-lock.yaml` を調べる）。指摘が1件でもあれば中止する。`--prod` は付けず、`devDependencies`（vite・`@tauri-apps/cli` など）も調べる。これらはビルドを動かす CI でシークレット（Updater の署名鍵）の近くで動き、配布物を作るため。
   - 修正版がまだ無い指摘は、リポジトリに置いた許可リスト（Rust は `.cargo/audit.toml` の `ignore`、JavaScript は `pnpm audit --ignore <GHSA の ID>` に渡す ID の一覧）に、理由と見直す期限を添えて載せたときだけ通す。許可リストの変更は PR で見る。
   - 検査は、その時点の脆弱性データベースと照合するので、同じタグを再実行すると結果が変わることがある（2 の「依存を固定したままビルドする」は、依存の版を固定する意味で、検査結果の固定ではない）。データベースやレジストリに届かず検査できなかったときも中止し（`--ignore-registry-errors` は使わない）、脆弱性による中止と区別して、つなぎ直してから再実行する。
   - `THIRD_PARTY_LICENSES.md` を、同梱するフォントのライセンスも含めて生成し（third-party-licenses.md 2章）、配布物に含める。
4. macOS：ビルド後にアドホック署名を行う（`codesign --force --deep -s - Yunagi.app`）。Apple Silicon では署名がないと起動できないため必須。
5. Updater 用の署名を、CI のシークレットに保管した秘密鍵で付与する。
6. SHA-256 を計算し、`.sha256` と `SHA256SUMS` を Release に添付する。`SHA256SUMS` に配布用鍵で署名し、`SHA256SUMS.sig` を添付する。
7. Homebrew tap のリポジトリに、新しいバージョンとハッシュを自動でプルリクエストする。

### action の固定

開発用 CI（`.github/workflows/ci.yml`）と配布用 CI のどちらも、ワークフローが使う action を次のとおり固定する（2026-11-05 決定。YUN-43）。タグは後から別のコミットに付け替えられ、乗っ取られた action が CI で動くおそれがある（配布用 CI ではビルドの署名鍵の近くで動く）ため、中身が変わらないコミットで指定する。

- `uses:` に書く action は、すべて 40 桁のコミットの SHA で指定し、同じ行の末尾に版のタグをコメントで併記する（例：`uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1`）。`actions/*` の公式の action も例外にしない。ブランチ名（`main`）、移動するタグ（`v7` のようなメジャー版だけのタグ）では指定しない。コメントの版は、`v7.0.1` のように完全な版で書く。
- 同じリポジトリの中の再利用ワークフロー・composite action（`./.github/...`）は、同じコミットのものを使うので、固定の対象外にする。Docker の image を使う `docker://` は、使うなら `sha256:` のダイジェストで指定する（今は使っていない）。
- SHA は、action のリポジトリに実在するタグから解決する（`gh api repos/<owner>/<repo>/git/ref/tags/<タグ>` で得た `object.sha` が、注釈付きタグ（`type` が `tag`）なら `gh api repos/<owner>/<repo>/git/tags/<その sha>` で、指す先のコミットの SHA を得る）。コメントの版と SHA が食い違わないよう、記憶や別の場所からの写しで書かない。
- 更新は Dependabot（`.github/dependabot.yml` の `github-actions`）に任せる。週に 1 回、新しい版があれば PR を出し、SHA とコメントの版を一緒に書き換える。公開から 14 日たった版だけを PR にする（`cooldown`。SECURITY.md 4.4 の依存の更新と同じ目安）。脆弱性の修正の PR は、この待ち時間に関係なく出る。人が手で足す・上げるときも、公開から 14 日を経た版を採る。Dependabot の PR は、1 週間分の更新を 1 つにまとめて出す（`groups`）。オーナーが、リリースの変更点（release notes）と差分を見てマージする。Dependabot の PR は、Linear の課題・ブランチ名（`yun-<番号>-...`）・`/code-review` の決まりの例外で、タイトルもブランチ名も既定のままにし、`/code-review` は行わない（CLAUDE.md の「ブランチと PR」にも書いた）。
- 固定していない action が入るのを防ぐため、リポジトリの設定（Settings → Actions → General）で「Require actions to be pinned to a full-length commit SHA」を有効にする。オーナーの設定の作業で、Linear の YUN-98 が追う。有効にするのは、この固定の PR（YUN-43）をマージし、ほかの開いている PR も main に追いつかせたあと（有効にすると、タグ指定のワークフローは実行されずに失敗するため）。配布用 CI を作るときも、この決まりに従う（YUN-97 の完了の条件に書いた）。
- Dependabot の対象は action だけで、Rust・JavaScript の依存の更新は含めない（SECURITY.md 4.4 のとおり、14 日の目安で別に更新する）。

## 4. インストールスクリプト

### 4.1 macOS・Linux（install.sh）

```sh
curl -fsSL https://<公式サイト>/install.sh | sh
```

スクリプトの動作：

1. OS とアーキテクチャを判定する。未対応の組み合わせは明確なメッセージで終了する。
2. 最新版（または `YUNAGI_VERSION` で指定した版）の配布物と `.sha256` を取得する。
3. ハッシュを照合し、一致しなければ中止する。
4. macOS：`~/Applications/Yunagi.app` に展開する（`/Applications` は権限が必要なため既定にしない）。Linux：`~/.local/opt/yunagi/Yunagi.AppImage` に AppImage を、`~/.local/opt/yunagi/bin/yunagi` に CLI を置き、デスクトップエントリを `~/.local/share/applications/io.github.misty-swan.yunagi.desktop` に作成する（ファイル名はアプリの識別子にそろえる。アーキテクチャ設計書 6.1）。アプリ本体の配置先は、データディレクトリ（アーキテクチャ設計書 6章。Linux は `~/.local/share/yunagi/`）とは別にする。
5. CLI を `~/.local/bin/yunagi` にリンクし、PATH に含まれていなければ追記方法を案内する。
6. 何をどこに置いたかを最後に一覧表示する。
7. Linux で Wayland のセッション（環境変数 `XDG_SESSION_TYPE` が `wayland` のとき、または `x11` 以外で `WAYLAND_DISPLAY` があるとき。操作仕様書 1章と同じ判定）のときは、最後に次の案内も表示する（Wayland では、OS 全体のショートカット Mod+Shift+D が効かないことがあるため。操作仕様書 1章「OS 全体の操作」、アーキテクチャ設計書 3.5）。デスクトップ環境のキーボード設定で、コマンド `~/.local/bin/yunagi show`（PATH に含まれていれば `yunagi show`）を好きなキーに割り当てる。設定の場所は、GNOME は「設定」→「キーボード」→「キーボードショートカット」→「カスタムショートカット」、KDE Plasma は「システム設定」→「キーボード」→「ショートカット」→「カスタムショートカットを追加」、Hyprland・Sway などのタイル型は設定ファイルのキー割当（`bind`）。同じ手順を利用者ガイド 2章にも載せる。案内には、`--note` の別のキー（ノートモードで開く `yunagi show --note`）と、ふだんのシェルの `PATH` を引き継がないキー設定では、フルパス `~/.local/bin/yunagi show`（展開した `$HOME` の実際のパス）で書く必要があることも添える（2026-11-04 決定。YUN-88）。`yunagi show`（アプリを前面に出す CLI。MCP・CLI 仕様書 3.3）は作ってあり、案内は設定の Wayland の文（操作仕様書 1章「OS 全体の操作」。文言は `src/lib/i18n_settings.ts` の `operation.global.wayland`）と食い違わせない（コマンド名は `yunagi show`）。install.sh 自体はまだ無い（Linear の YUN-95 が install.sh・install.ps1 を作り、この案内を出すことを完了の条件に書いた）。
   - `yunagi show` がアプリを探す場所（MCP・CLI 仕様書 3.3）は、この章の配置に合わせている：Linux は `~/.local/opt/yunagi/bin/yunagi` の隣の `../Yunagi.AppImage`、macOS は `yunagi` を含む `Yunagi.app`、Windows は `bin\yunagi.exe` の隣の `..\Yunagi.exe`。配置をこれと変えるときは、探す場所（`crates/cli/src/show.rs` の `locate`）も変える。tar.gz・.deb の置き場所は、まだ決めていない（決めるときに足す）。

スクリプトは `sh` 互換で書き、`set -eu` で途中失敗時に止める。スクリプト自体も公開リポジトリで公開し、内容を確認してから実行する方法（ダウンロードして読む手順）を導入ガイドに載せる。

### 4.2 Windows（install.ps1）

```powershell
irm https://<公式サイト>/install.ps1 | iex
```

動作は install.sh に準じる。`Yunagi_<ver>_<arch>-setup.exe` を取得してハッシュを照合し、`/S`（サイレント）で実行する。配置先は `%LOCALAPPDATA%\Programs\Yunagi\`、スタートメニューにショートカットを作成し、ユーザー環境変数の PATH に `bin` を追加する。

### 4.3 Homebrew（macOS）

```sh
brew install <tap名>/yunagi
```

tap 内の Cask でアプリと CLI を配置する。アプリは Cask の既定どおり `/Applications/Yunagi.app` に置き、CLI は Cask の `binary` で Homebrew の bin にリンクする。

Cask はダウンロードしたアプリに隔離属性（`com.apple.quarantine`）を付けるため、アドホック署名のままでは初回起動が Gatekeeper に止められる見込みである（要確認：Homebrew の挙動は変わりうるため、実機で確認する）。Cask の `postflight` で隔離属性を外す。外せない場合は、導入ガイドに対処（`xattr -dr com.apple.quarantine /Applications/Yunagi.app`）を載せる。

### 4.4 アンインストール

`yunagi uninstall` で、アプリ本体と CLI のリンク、Linux ではデスクトップエントリ（4.1）を削除する。データディレクトリは既定では残し、`--purge` 指定時のみ削除する。

## 5. 自動更新

- 更新の自動確認は既定で有効とし、設定（一般 →「更新の自動確認」）で無効にできる。確認時に送る情報は版・OS・アーキテクチャのみ（配布ホストには IP アドレスとアクセス日時が記録される）。
- 有効な場合、アプリ起動時と以後24時間ごとに `latest.json` を確認する。
- 新しい版があれば、サイドバー下端の「New version」で控えめに通知し（画面設計書 3章）、利用者の操作（「New version」を選んで「更新して再起動」、またはパレットの「更新して再起動」コマンド）で更新する（作業中に勝手に再起動しない）。
- 更新前にデータベースのバックアップを作成する。マイグレーションが失敗した場合は自動でバックアップに戻してデータを変更せず、エラー画面で旧版の再導入方法（`YUNAGI_VERSION=<旧版>` を付けてインストールスクリプトを実行）を案内する。データベースが新しい版で作られていて開けない場合（旧版を入れ直したときなど）は、新しい版の入れ方（`YUNAGI_VERSION` を付けずに実行）を案内する（画面は画面設計書 10.1）。

## 6. 鍵の管理

### 6.1 更新署名鍵

この鍵が漏えいすると、全利用者に不正な更新を配布できてしまうため、最重要の秘密情報として扱う。

- 秘密鍵は CI のシークレットとオフラインのバックアップ（暗号化した外部媒体）の2か所にのみ置く。
- 鍵のパスフレーズはパスワードマネージャーで管理し、鍵ファイルとは別に保管する。
- 漏えいが疑われる場合の手順：新しい鍵を生成 → 新しい公開鍵を含む版を、旧鍵で署名して最後に1回だけ配布 → 旧鍵の失効を、公式サイトのお知らせと GitHub Security Advisory で告知する（release/SECURITY.md 6章）。

### 6.2 配布用鍵（`SHA256SUMS.sig`）

- 更新署名鍵とは別に生成し、同じ方法（CI のシークレットと、暗号化したオフラインのバックアップの2か所。パスフレーズは別に保管）で管理する。
- 漏えいが疑われる場合は、新しい鍵で `SHA256SUMS` に署名し直し、公式サイトと公開リポジトリの公開鍵を差し替えて、6.1 と同じ方法で告知する。

## 7. リリース手順チェックリスト

- [ ] CHANGELOG.md を更新した
- [ ] バージョン番号を `Cargo.toml`・`tauri.conf.json`・`package.json` で揃えた
- [ ] テスト計画書のリリース前テストを完了した
- [ ] `THIRD_PARTY_LICENSES.md` を生成し、許可外ライセンスと既知脆弱性が無いことを確認した
- [ ] 型チェックの道具（svelte-check）の新しい版が TypeScript 7 系に正式に対応したかを、アーキテクチャ設計書 3.2 の目安で確かめた（対応していれば移行を判断する）
- [ ] タグを作成して push した
- [ ] CI の全ジョブが成功し、配布物・`.sha256`・`.sig`・`SHA256SUMS`・`SHA256SUMS.sig` が揃っている
- [ ] `https://<公式サイト>/latest.json` を新しい版に更新した
- [ ] 配布する全 OS・アーキテクチャで、インストールスクリプトによる新規導入と、旧版からの自動更新を確認した（検証できなかったものは「試験提供」と明記した）
- [ ] Homebrew tap の更新がマージされ、Cask で導入したアプリが起動することを確認した
- [ ] 公式サイトのダウンロード案内と変更履歴を更新した
