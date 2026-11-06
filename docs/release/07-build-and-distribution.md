# ビルド・配布手順書

## 1. 方針

- 有料の開発者登録（Apple Developer Program、Windows のコード署名証明書）を使わずに配布する。
- 主な導入方法は、ターミナルからの1行インストール。macOS は Homebrew tap も提供する。
- アプリの自動更新は Tauri Updater を使い、Yunagi 独自の鍵で署名する。
- 配布物の実体は GitHub Releases に置き、利用者への案内は公式サイトに一本化する（Homebrew tap は公式サイトから案内する補助経路）。公式サイトは Cloudflare Pages に置く（2章「公式サイトの置き場所」）。要件定義書 9章の「配布経路は公式サイトのみ」は「ストアでは販売しない」の意味である。
- **本体のソースは、今のリポジトリ `misty-swan/yunagi` をそのまま公開する**（2026-11-06 オーナー決定。YUN-103）。配布・Issue・脆弱性報告も、同じ公開リポジトリで受ける（インストールスクリプト、CHANGELOG、SECURITY.md もここに置く）。本書で「GitHub」と書くときはこの公開リポジトリを指す。
  - 本体ソースのライセンスは **MIT と Apache-2.0 のデュアル**（利用する人がどちらかを選べる）。LICENSE ファイルを置くのは YUN-109（third-party-licenses.md 3章）。
  - 公開への切り替えは、公開前の点検（秘密情報・公開してよくないファイル・LICENSE・公開向けの README など）を済ませてから、オーナーが行う（YUN-109）。切り替えまでは、リポジトリは非公開のままで、この書の「公開した時点で〜」と書いた動きは、まだ確かめられていない。
- 全配布物の SHA-256 ハッシュと、その署名を公開する（2章）。

## 2. 配布物

| OS | アーキテクチャ | 配布物 | CLI の配置 |
| --- | --- | --- | --- |
| macOS | arm64、x86_64 | `Yunagi_<ver>_<arch>.app.tar.gz` | アプリに同梱した `yunagi` を `~/.local/bin` にリンク（Homebrew の場合は Cask の `binary` で Homebrew の bin にリンク） |
| Linux | x86_64、arm64 | `Yunagi_<ver>_<arch>.AppImage`、`yunagi-cli_<ver>_<arch>.tar.gz`、`.tar.gz`、`.deb`（任意） | `~/.local/opt/yunagi/bin/yunagi` を `~/.local/bin/yunagi` からリンク |
| Windows | x86_64、arm64 | `Yunagi_<ver>_<arch>-setup.exe`（NSIS、ユーザー単位インストール） | `%LOCALAPPDATA%\Programs\Yunagi\bin` を PATH に追加 |

- CLI は GUI とは別のコンソール用バイナリ `yunagi`（Windows は `yunagi.exe`）として同梱する。
- **配布物の名前と中身**（2026-11-05 決定。YUN-95。インストールスクリプトと配布用 CI（YUN-97）が食い違わないための取り決め）：
  - `<ver>` は `1.2.3` の形（`v` を付けない）。Release のタグは `v<ver>`。ファイルは `<配布元>/v<ver>/<ファイル名>` に置く（`<配布元>` は `https://github.com/misty-swan/yunagi/releases/download`。公開リポジトリの Release のダウンロード先。2026-11-06 決定。YUN-103。たとえば `https://github.com/misty-swan/yunagi/releases/download/v1.2.3/Yunagi_1.2.3_arm64.app.tar.gz`）。
  - `<arch>` は `x86_64` か `arm64` の 2 つだけ。Tauri がビルドで付ける名前（`amd64`・`aarch64`・`x64` など）は、配布用 CI がこの名前に付け替えてから Release に置く（付け替えは 3.3 で作った。Release に置くのは YUN-101）。インストールスクリプトは `uname -m` の `aarch64` を `arm64` に読み替える。
  - 配布物ごとに `<ファイル名>.sha256` を添える。中身は `sha256sum` の出力と同じ `<64 桁の 16 進のハッシュ>  <ファイル名>` の 1 行。スクリプトは最初の欄だけを読む。
  - Linux の `yunagi-cli_<ver>_<arch>.tar.gz` は CLI だけの配布物で、中身はルートに置いた実行ファイル `yunagi` 1 つ（AppImage の中の CLI は外から取り出せないため、別に出す。`install.sh` が使う）。
  - macOS の `.app.tar.gz` は、ルートに `Yunagi.app` を持ち、CLI は `Yunagi.app/Contents/MacOS/yunagi` に置く。
  - Windows の `-setup.exe` は、`bin\yunagi.exe` と `Yunagi.exe` を含む（インストール先の `bin\yunagi.exe` が `install.ps1` の確認の対象。4.2）。
- アプリ内自動更新の対象は、macOS の `.app.tar.gz`、Linux の AppImage、Windows の NSIS 版に限る。Linux の `.tar.gz`・`.deb` は自動更新の対象外で、再導入で更新する（利用者ガイドに明記する）。
- 配布するアーキテクチャは、テスト計画書 3章の対象環境と1対1で対応させる。リリース前に検証できなかった組み合わせは、ダウンロード案内とリリースノートで「試験提供（動作未保証）」と明記する。

各配布物に対応する `.sha256` と、Tauri Updater 用の署名ファイル（`.sig`）を同時に公開する。あわせて、全配布物のハッシュをまとめた `SHA256SUMS` と、その minisign 署名 `SHA256SUMS.sig` を公開する。`SHA256SUMS.sig` の署名には、Updater 鍵とは別の配布用鍵を使う。配布用の公開鍵は公式サイトと公開リポジトリの両方に掲載し、検証手順を導入ガイドに載せる。更新情報 `latest.json` は、公式サイト（Cloudflare Pages）の `https://getyunagi.com/latest.json` に置き、取り下げ時にすぐ前の版へ戻せるようにする（下の「公式サイトの置き場所」）。

### 公式サイトの置き場所（2026-11-06 決定。YUN-103）

導入案内・ダウンロード・`latest.json`・導入スクリプト（`install.sh`・`install.ps1`）・配布用の公開鍵・お知らせを載せる公式サイトは、**Cloudflare Pages**（無料枠）に置く。ドメインは **`getyunagi.com`**（Cloudflare Registrar で取得済み。DNS も Cloudflare）。公式サイトの URL は `https://getyunagi.com`。商標（J-PlatPat で「YUNAGI」を第9類・第42類で検索して該当なし。2026-11-06）の確認と、設計書・コード・導入スクリプトの仮の表記の置き換えは、YUN-11 で済んだ（本書の `<配布元>` は、上の `https://github.com/misty-swan/yunagi/releases/download` を指す本書の書き方で、仮の表記ではない）。問い合わせ先は `support@getyunagi.com`、脆弱性の報告先は `security@getyunagi.com`。サイトの作り方（静的サイトの道具・日英の切り替え・ページ構成）は YUN-107 で決める。

**サイトを実際に公開するまでの依存**（この間、`getyunagi.com` は何も表示せず、導入コマンドは動かない）：

| 仕事 | 課題・担当 |
| --- | --- |
| Cloudflare Pages のプロジェクトの作成、リポジトリとの結び付け、`getyunagi.com` の割り当て、サイトの作成と公開、`install.sh`・`install.ps1` の掲載 | YUN-107 |
| `latest.json` と配布物を作って公開する配布用 CI（導入コマンドが動くのは、これで最初のリリースを出したあと） | YUN-101 |
| `support@getyunagi.com`・`security@getyunagi.com` の受け口（Cloudflare の Email Routing で、オーナーの受信箱へ転送する設定） | オーナーの作業（YUN-11 で依頼） |

- **選んだ理由**：①独自ドメインの結び付けが簡単（ドメインの DNS も Cloudflare にあるので、記録は Cloudflare が自動で作る）。②リポジトリが非公開でも公開できる。③`latest.json` を含むサイトを、操作 1 つで前の公開済みの状態へ戻せる（下の「取り下げ時の戻し方」）。
- **配布物の実体は置かない**。Pages には 1 ファイル 25 MiB の上限があり（Cloudflare Pages の limits）、AppImage や Windows の配布物が収まらないため。配布物は GitHub Releases（`<配布元>`）に置き、Pages には置かない。サイトが置くのは、小さい文書・スクリプト・`latest.json`・公開鍵だけ。
- **独自ドメインの結び付け**（オーナーの作業。YUN-107 でサイトを公開するときに行う）：
  - Pages のプロジェクトの「Custom domains」に `getyunagi.com` を追加する。apex ドメイン（`www` の付かないもの）を結び付けるには、そのドメインが、Pages のプロジェクトと同じ Cloudflare アカウントの zone（DNS の管理単位）であること。`getyunagi.com` は Cloudflare Registrar で取得し DNS も Cloudflare なので満たす。CNAME などの DNS の記録は Cloudflare が自動で作る。
  - **証明書の発行を妨げる CAA 記録が無いことを確かめる**：`dig CAA getyunagi.com`（または Cloudflare の DNS の画面）で見る。CAA 記録が 1 つも無ければ、何もしなくてよい（記録を足さない）。1 つでもある場合は、Pages が使う認証局（`letsencrypt.org`・`pki.goog; cansignhttpexchanges=yes`・`ssl.com`）の発行を許す記録（`issue` と `issuewild`）が含まれていないと、ドメインの追加が検証で止まる。そのときだけ、足りない認証局の記録を足す（ほかの用途で別の認証局の記録を置いているなら、それは消さない）。ここに書いた認証局の名前は 2026-11-06 時点の Cloudflare の案内（https://developers.cloudflare.com/pages/configuration/custom-domains/ の「Known issues」）の写しで、変わりうるので、足すときはそのページの最新の一覧を見る。確かめるのは、Cloudflare の DNS の画面に出る記録（ダッシュボードで足した分も含む）と `dig` の結果の両方（Cloudflare Pages の custom domains の「Known issues」）。
  - `www.getyunagi.com` の扱い（apex へ転送するか）は YUN-107 で決める。
- **サイトの自動公開は、Direct Upload を GitHub Actions から行う方針**（推奨案。Cloudflare の Git 連携は使わない）：
  - 流れ：GitHub Actions のワークフローが、サイトの元（`site/` など。置き場は YUN-107）をビルドして `wrangler pages deploy <出力先> --project-name=<プロジェクト名> --branch=<本番ブランチ>` で上げる。Cloudflare の公式の手順（`cloudflare/wrangler-action`）に沿う。`--branch` には Pages のプロジェクトの本番ブランチ名を指定する（それ以外の名前で上げたものはプレビューになり、本番にならず、戻し先にもならないため）。
  - 理由：①ビルドを自分たちの CI（lock ファイル厳守・action の SHA 固定。3章「action の固定」）で行える。Git 連携だと Cloudflare 側のビルド環境に任せ、固定の決まりが及ばない。②いつ公開するかを自分たちで決められる（`latest.json` は Release の配布物がそろってから公開する。7章）。Git 連携では、サイトのファイルを main に入れると自動で公開されてしまう。③`latest.json` を、リポジトリに書き戻さず、配布用 CI（YUN-101）が作ったものとしてそのまま上げられる。
  - 注意：Pages のプロジェクトは、Git 連携と Direct Upload を後から切り替えられない（Cloudflare の公式の案内）。ここで Direct Upload に決めるので、プロジェクトは **Direct Upload のプロジェクトとして作る**。
  - 秘密：Cloudflare の API トークンは、「Account → Cloudflare Pages → Edit」だけの権限で作り、GitHub Actions の secrets（`CLOUDFLARE_API_TOKEN`・`CLOUDFLARE_ACCOUNT_ID`）に置く。公開用のワークフローだけが読める形にする。リポジトリは公開されるので、**フォークからの PR（`pull_request`・`pull_request_target`）の実行にはこの secrets（と署名鍵の secrets）を渡さない**：サイトの公開と署名は、タグの push・main への push・手動の実行（`workflow_dispatch`）でだけ動かし、GitHub の Environment にブランチ・タグの制限をかけて secrets をそこに置く。試運転（`pull_request`）は secrets を使わない（今の `release.yml` もそうしている）。今の `release.yml` には `environment:` も、手動の実行を main やタグに限る条件も無い。そのため、YUN-101・YUN-107 がこの制限を作るまで、リポジトリ全体の secrets として Cloudflare のトークンや署名鍵を**置かない**（リポジトリの公開は YUN-109 で、それより前でも後でも同じ）。細部は YUN-107・YUN-101 で決める（YUN-101 の Linear のコメントに残した）。`wrangler-action` も 3章「action の固定」に従って SHA で固定する。Cloudflare のアカウントには 2 要素認証をかける：アカウントを奪われると、導入スクリプトと `latest.json` を差し替えられる（更新自体は署名を検証するので不正な更新は入らないが、導入スクリプトは署名で守られないため、守りの要になる。SECURITY.md 4.3）。
- **`latest.json` の送り方**：`/latest.json` は、古い内容が使われ続けないよう、サイトの `_headers` ファイルで `Cache-Control: no-cache` を明示する（Pages の `_headers` で付ける。YUN-107 が作り、公開後に `curl -I` で確かめる）。
- **取り下げ時の戻し方**（サポート・障害対応手順 5章が参照する）：次の 2 つのどちらかで、`latest.json` を前の版に戻す。急ぐときは (1)。
  1. **前の公開済みのデプロイに戻す**（Pages の「Rollback」。Cloudflare の公式の案内 Rollbacks）：Cloudflare のダッシュボードで対象の Pages のプロジェクトを開き、「Deployments」の「All deployments」から、戻したい本番のデプロイの「⋯」メニュー →「Rollback to this deployment」を選んで確認する。**戻し先は「直前のデプロイ」ではなく、`latest.json` が良い版（取り下げる版の 1 つ前）を指している最後の本番のデプロイ**を選ぶ（取り下げる版の後に、サイトだけを直したデプロイがあると、直前のものにも取り下げる版の `latest.json` が入っている）。各デプロイの `latest.json` は、リリースごとに版を控えておく（7章）か、そのデプロイの URL で見て確かめる。**戻したあとは、公開用のワークフローを止める**（Actions の画面で無効にするか、実行を保留する）：main から新しく上げる実行が、戻した状態を取り下げる版で上書きしてしまうため。原因を直して、良い版の `latest.json` を作れるようになってから再開する。本番のサイトが**すぐに**その時点の内容に変わる。戻せるのは、ビルドに成功した**本番**のデプロイだけ（プレビューは戻し先にならない）。戻るのは `latest.json` だけでなくサイト全体（導入スクリプトなども、そのデプロイの時点の内容）。新しいデプロイに進み直すこともできる。
  2. **ファイルを差し替えて再デプロイする**：前の版の `latest.json` を元の内容で作り直し（Release の配布物と `.sig` から、配布用 CI の手順で再生成する）、公開用のワークフローを手動で実行して上げる。数分かかるが、サイトのほかの部分は今のままにできる。
  - どちらの後も、`curl -fsS https://getyunagi.com/latest.json` の `version` が前の版に戻ったことを確かめる。すでに新しい版へ更新した利用者の手元は戻らない（アプリは古い版へ自動では戻さない）。広がりを止めるための手段である。GitHub の Release の取り下げ（ドラフトへ戻す・削除）は別に行う。手順は、サポート・障害対応手順 5章。
  - 直前の本番のデプロイが残っていること（戻し先があること）を、リリースのたびに確かめる（7章）。

### アプリアイコン

アプリアイコンの絵は、デザインガイド 8章の「夕凪版」。画面の左上のロゴ（`src/shell/Logo.svelte`）の絵を 1024px に拡大したもので、色・水平線の位置（58%）・半分沈んだ太陽・海面の光の道・角丸（一辺の 25%）はロゴと同じにする（2026-11-05 決定。YUN-33）。Design キャンバスの元の図は取り出せないため、ロゴの図形を元に組み直した。

- **元画像（正本）**：`src-tauri/icons/source/` に置く。
  - `app-icon.svg`：1024×1024 の全面版。Windows・Linux 用。
  - `app-icon-macos.svg`：同じ絵を、四方に透明の余白（約 10%。一辺 824px の角丸を中央に置く）を残して縮めた版。macOS 用。macOS の Dock・Finder は角丸の下地を自分では付けず、アイコンの絵のとおりに出すため、余白が無いと他のアプリより大きく見える。
  - `app-icon.png`・`app-icon-macos.png`：上の SVG を 1024px の PNG にしたもの（`pnpm tauri icon` の入力）。
- **生成物**：`src-tauri/icons/` の `icon.ico`（Windows。16・24・32・48・64・256px）、`icon.icns`（macOS）、`icon.png`（512px）、`32x32.png`、`128x128.png`、`128x128@2x.png`。`tauri.conf.json` の `bundle.icon` が指すのはこの 6 つ。`icon.png`（512px）を先頭に置く：Tauri は窓のアイコンを `bundle.icon` の先頭の画像から作るため、先頭が 32px だと Linux の高解像度の画面でぼやける。
- **食い違いの検査**：`install/appIcon.test.ts`（`pnpm check` に含まれる）が、SVG の色・グラデーションの位置・水平線と太陽と光の帯と角丸の大きさが `Logo.svelte` を拡大したものと同じこと、macOS 版が全面版の定義と絵をそのまま縮めて置いただけであること（縮め方は 824px・余白 100px）、元 PNG と生成 PNG の大きさ、`icon.ico` の絵のサイズが 16・24・32・48・64・256px であること、`icon.icns` の形式を確かめる。ロゴの図形や色を変えて SVG を直し忘れると、ここで落ちる。生成物が古いこと・全面版と macOS 版の取り違えは検査できない（手順 3 で取り違えると、macOS で大きすぎ、または Windows で余白つきに見える）。スクリプトは取り違えないように書いてあるので、手で作り直さず、スクリプトを使う。作り直したら 08 4.10 の見た目を確かめる。
- **開発時の窓のアイコン**：先頭の `icon.png` は全面版なので、`pnpm tauri dev` で動かした macOS の Dock には、余白の無い絵が出ることがある（ビルドした `.app` の Dock・Finder は `icon.icns` を使う）。開発時だけの見た目で、直さない。
- **角丸**：macOS 版も、縮める前の角丸（一辺の 25%）のままにする（ロゴと同じ形に保つため）。Apple の標準の格子（一辺 824px の約 22.5%、連続した曲線）とは少し違うが、実機で見て違和感が大きければ、macOS 版だけ角丸を直す（08 4.10）。
- **作り直す手順**（SVG を直したとき）：リポジトリ直下で `sh src-tauri/icons/source/generate.sh` を実行する。事前に `rsvg-convert`（SVG を PNG にする。librsvg に入っている）と `pnpm`（`pnpm tauri icon` が使う）が要る。スクリプトは次の 3 つをする。手で行うときも同じ順にする。
  1. 2 つの SVG を 1024px の PNG にする（`rsvg-convert -w 1024 -h 1024`）。
  2. `pnpm tauri icon` を、全面版の PNG と macOS 版の PNG のそれぞれに、別々の一時フォルダを出力先にして実行する（Tauri 公式の生成。各サイズの PNG・ico・icns が出る）。
  3. 全面版の出力から `icon.ico`・`icon.png`・`32x32.png`・`128x128.png`・`128x128@2x.png` を、macOS 版の出力から `icon.icns` だけを、`src-tauri/icons/` にコピーする。`tauri.conf.json` が使わない Android・iOS・Windows ストア用のファイルは置かない。
  - `src-tauri/icons/` に `source/` を置いても、配布物には入らない（`bundle.icon` が列挙したファイルだけが入る）。
- 絵の確認：16〜32px では、太陽は空の橙と色が近く見分けにくいが、空と海の明暗の境目と水平線の横線で夕凪の絵と分かる。ロゴ（24px）と同じ見え方なので、このままとする。
- Linux の AppImage・デスクトップエントリには、いまのところアイコンを付けない（4.1 の決まりのまま）。実機での見た目は 08 4.10 で確かめる。

## 3. ビルド（CI）

GitHub Actions で OS ごとのマトリクスビルドを行う。ワークフローは `.github/workflows/release.yml`（開発用 CI の `ci.yml` とは別）。作る範囲は 2 つの課題に分けている（2026-11-05 決定）：1〜3 と 6 の `.sha256` までを YUN-97（検査・ビルド・配布物の名前と中身。3.1〜3.3）、4・5 と 6 の `SHA256SUMS` 以降・7 を YUN-101（署名と公開。3.4。秘密鍵（YUN-100）と、リポジトリの公開（YUN-109）・公式サイトの公開（YUN-107）が済んでから動かせる。置き場所は決まっている：1章・2章）が作る。

1. タグ `v<ver>` の push を契機に起動する。タグの `<ver>` は、直下の `Cargo.toml` の `[workspace.package]` の `version` と同じでなければ中止する（版は 1 か所にだけ書く。README.md）。
2. 依存を固定したまま（lock ファイルを厳守）ビルドする。
3. 依存の既知脆弱性の検査と、許可外ライセンスの検出（third-party-licenses.md 2章）を行い、問題があれば中止する。同梱するフォントのファイルの SHA-256 がデザインガイド 4章に書いた値と一致し、ライセンス文書（UDEV Gothic HS は `LICENSE`・`LICENSE_JetBrainsMono`・`LICENSE_BIZUDGothic`、Inter は `LICENSE.txt`）がそろっていることも確かめ、違えば中止する。脆弱性の検査は次のとおり。
   - Rust：`cargo audit`（`Cargo.lock` を調べる）。脆弱性（vulnerability）の指摘が1件でもあれば中止する。メンテナンス終了（unmaintained）・健全性の問題（unsound）・取り下げ（yanked）の警告は、ジョブのログに警告の注釈として残すだけで中止しない（下の「警告の扱い」）。
   - JavaScript：`pnpm audit`（`pnpm-lock.yaml` を調べる）。指摘が1件でもあれば中止する。重大度のしきい値（`--audit-level`）は付けない（low も止める。`--audit-level` は表示する指摘の絞り込みで、付けるとリリースを止めるべき指摘を見落とすため）。`--prod` は付けず、`devDependencies`（vite・`@tauri-apps/cli` など）も調べる。これらはビルドを動かす CI でシークレット（Updater の署名鍵）の近くで動き、配布物を作るため。
   - 修正版がまだ無い指摘は、リポジトリに置いた許可リストに、理由と見直す期限を添えて載せたときだけ通す。許可リストの変更は PR で見る。置き場所と書き方は下の「許可リスト」。
   - 検査は、その時点の脆弱性データベースと照合するので、同じタグを再実行すると結果が変わることがある（2 の「依存を固定したままビルドする」は、依存の版を固定する意味で、検査結果の固定ではない）。データベースやレジストリに届かず検査できなかったときも中止し（`--ignore-registry-errors` は使わない）、脆弱性による中止と区別して、つなぎ直してから再実行する。区別は `scripts/release/audit.ts` が道具の出力（JSON）を読んで行い、終了コード 1 = 脆弱性の指摘、2 = 検査できなかった、とメッセージ（「脆弱性の指摘が n 件ある」「検査できなかった（脆弱性の指摘ではない）」）で分ける。結果が読めても道具の終了コードが 0 でなければ、「指摘なし」とは言わない。
   - `THIRD_PARTY_LICENSES.md` を、同梱するフォントのライセンスも含めて生成し（third-party-licenses.md 2章）、配布物に含める。
4. macOS：ビルド後にアドホック署名を行う（`codesign --force --deep -s - Yunagi.app`）。Apple Silicon では署名がないと起動できないため必須。（YUN-101 で作る。3.4）
5. Updater 用の署名を、CI のシークレットに保管した秘密鍵で付与する。（YUN-101 で作る。3.4）
6. SHA-256 を計算し、`.sha256` と `SHA256SUMS` を Release に添付する。`SHA256SUMS` に配布用鍵で署名し、`SHA256SUMS.sig` を添付する。（`.sha256` を配布物ごとに作るところまでは YUN-97 で作った。3.3。`SHA256SUMS` とその署名・Release への添付は YUN-101 で作る。3.4）
7. Homebrew tap のリポジトリに、新しいバージョンとハッシュを自動でプルリクエストする。（YUN-101 で作る。3.4）

### 3.1 検査（許可リスト・ライセンス・フォント）

2026-11-05 決定（YUN-97）。検査は `release.yml` の `verify` ジョブが順に行い、1 つでも失敗するとビルドの前に止まる。手順の名前（ジョブのログの見出し）で、どの検査で止まったかが分かる。

**許可リスト**

- **正本は `scripts/release/audit-allowlist.json`。** Rust（`rust`）と JavaScript（`javascript`）の欄に、指摘の ID（`RUSTSEC-…`・`GHSA-…`）、理由（`reason`。なぜ今は直せないか・なぜ影響しないか）、外す条件（`remove_when`）、見直す期限（`expires`。`YYYY-MM-DD`）を 1 件ずつ書く。
- **検査の道具が読む設定にも同じ ID を書く。** Rust は `.cargo/audit.toml` の `ignore`、JavaScript は `pnpm-workspace.yaml` の `auditConfig.ignoreGhsas`（pnpm 12 が `pnpm audit` で読む）。JavaScript の許可に `pnpm audit --ignore <GHSA>` は使わない：付けると pnpm が `pnpm-workspace.yaml` に ID を書き足してしまい（2026-11-05 に確かめた）、理由と期限の無いまま許可が増えるため。2 か所の ID が正本と同じ集まりであることは、`pnpm check:allowlist` と `scripts/release/allowlist.test.ts` が確かめる（どちらかだけ書き換えると落ちる）。道具が読む設定を使うので、手元の `cargo audit`・`pnpm audit` も CI と同じ結果になる。
- **期限は `pnpm check:allowlist`（`scripts/release/check-allowlist.ts`）が確かめる。** `cargo audit`・`pnpm audit` は期限を見ないため、別の手順で見る。`pnpm check` に入れてあるので、手元と開発用 CI で毎回、今日の日付（UTC）で確かめ、期限の翌日から全 PR が落ちる（配布用 CI の `verify` も最初に確かめる）。落ちたら、まだ修正版が無いなら理由を見直して期限を延ばし、修正版があれば依存を上げて許可リストから外す。期限の置きっぱなしを防ぐため、期限は今日から 190 日（約 6 か月）先までしか置けない。ID の形・理由と外す条件が空でないこと・ID の重複も同じ手順で確かめる。
- **いまの許可リスト**（2026-11-05。YUN-83）：Rust は `tauri-plugin-updater` が固定している `zip` 4.6.1 の脆弱性 4 件（RUSTSEC-2026-0367・0368・0369・0429。修正は `zip` 9.0.0 以上。Windows の zip 形式の更新の展開だけが使う部品で、Yunagi は NSIS の `.exe` で配るので通らず、通る場合も署名の検証の後。プラグインが `zip` 9 以上に上がったら外す。見直す期限 2027-02-05）。JavaScript は `deepmerge`（GHSA-gcqx-w48m-w2h4。`@sveltejs/vite-plugin-svelte` の依存。修正版なし。開発時とビルド時にだけ使い、配布物に入らない。見直す期限 2027-02-05）。

**警告の扱い**

- `cargo audit` の警告（メンテナンス終了・健全性の問題 `unsound`・取り下げ `yanked`）は、中止せず、ジョブの注釈（警告）に出す。`unsound` も同じ扱いにする：いま出ている `glib` 0.18.5（RUSTSEC-2024-0429。Tauri の GTK 3 の連携が固定していて上げられない）のように、自分たちでは直せないものが多く、中止にすると許可リストが警告で埋まるため。代わりに、リリースのたびに 7 章の確認表で、警告の一覧に新しいもの（特に `unsound`）が増えていないかを人が見る。
- 取り下げ（`yanked`）：14 日の待ちのために意図して一つ前の版へ下げた依存（`clipboard-win` 5.4.1 など。SECURITY.md 4.4）は、取り下げの警告が出る。新しい版が 14 日を経たら上げる。

**許可外ライセンス**

- 許可するライセンスは `scripts/release/license-policy.json` の `allowed`（third-party-licenses.md 2章の一覧：MIT・Apache-2.0・BSD-2-Clause・BSD-3-Clause・ISC・Zlib・MPL-2.0・BSL-1.0・CDLA-Permissive-2.0・Unicode-3.0。後ろの 3 つは YUN-96 で加えた）。ライセンスの式（`MIT OR Apache-2.0` など）は、`OR` はどれか 1 つ、`AND` はすべてが許可に入っていれば通る。
- Rust は `cargo-about`（`about.toml`）で調べる。配布する 6 組の target の依存だけを数え、ビルド時とテスト時にだけ使う依存（ビルドスクリプト・proc-macro・dev-dependencies）は実行ファイルに入らないので数えない。自分たちのクレートは数えない。JavaScript は `pnpm licenses list --prod`（配布物に入る `dependencies` だけ。脆弱性の検査と違い `devDependencies` は数えない：ライセンスは配布する物にだけ付くため）。同梱するフォントは、知らないフォルダ・ファイル（デザインガイド 4章に無いもの）と、足りないライセンス文書を `scripts/release/licenses.ts` が見つける。
- **許可するか決まっていないライセンス**（`pending`）：新しい依存が一覧に無いライセンスを連れてきたとき、オーナーが決めるまでの置き場で、ふだんは空（2026-11-06 の YUN-96 で、それまで入っていた BSL-1.0・CDLA-Permissive-2.0・Unicode-3.0 を `allowed` へ移した）。入っている間は、配布用 CI のこの検査は落とす（タグの push でも試運転でも。試運転の PR は、オーナーが決めて `pending` が空になるまで赤のままで、その後ろの `THIRD_PARTY_LICENSES.md` の生成も走らない。`release.yml` の「Licenses (disallowed)」に `continue-on-error` は付けない）。`THIRD_PARTY_LICENSES.md` の生成は、`about.toml` の `accepted` に `pending` も入れて受け入れ、決まるまでも一覧を作れるようにしている（`accepted` は `allowed` と `pending` を合わせたものと同じに保つ。試験が確かめる）。決まったら、許可するものは `allowed` へ移し、許可しないものは依存を外して `pending` から消す。
- **フォントのファイルの SHA-256 とライセンス文書**は `src/styles/fonts.test.ts`（`pnpm check` に含まれ、デザインガイド 4章の値と照合する）を `verify` ジョブでも動かす。違えば中止する。
- `THIRD_PARTY_LICENSES.md`：`node scripts/release/licenses.ts generate <出力先>`。フォント → JavaScript の依存 → Rust の依存の順に、同じライセンス文書ごとにまとめて載せる。`verify` ジョブが作って成果物にし、ビルドの各ジョブが `src-tauri/THIRD_PARTY_LICENSES.md` に置いて、`bundle.resources`（`src-tauri/release/common.json`）で配布物に入れる（Linux の AppImage の中は `usr/lib/Yunagi/THIRD_PARTY_LICENSES.md` に入ることを確かめた）。アプリの「設定 → このアプリについて」でこのファイルを表示する画面は、まだ無い（画面の課題で作る。third-party-licenses.md 2章）。

**使う道具の版**：`cargo-audit` 0.22.2、`cargo-about` 0.9.2（最新の 0.9.3 は公開から 14 日たっていないので、14 日を経た中で最も新しい版にした。`cargo install --locked` で入れる）。上げるときは、公開から 14 日を経た版を採る（SECURITY.md 4.4）。

### 3.2 ビルド（マトリクス）

2026-11-05 決定（YUN-97）。`build` ジョブが、検査（`verify`）の成功の後に、次の 6 組を並べて動かす（`fail-fast: false`。1 組の失敗で他の組を止めない）。組み合わせは `scripts/release/matrix.ts` が作る。

| 名前 | ランナー | target | `--bundles` |
| --- | --- | --- | --- |
| linux-x86_64 | `ubuntu-24.04` | `x86_64-unknown-linux-gnu` | `appimage` |
| linux-arm64 | `ubuntu-24.04-arm` | `aarch64-unknown-linux-gnu` | `appimage` |
| macos-arm64 | `macos-15` | `aarch64-apple-darwin` | `app` |
| macos-x86_64 | `macos-15`（交差ビルド） | `x86_64-apple-darwin` | `app` |
| windows-x86_64 | `windows-2025` | `x86_64-pc-windows-msvc` | `nsis` |
| windows-arm64 | `windows-2025`（交差ビルド） | `aarch64-pc-windows-msvc` | `nsis` |

- 手順：①Rust・pnpm・Node.js を入れる（版は `rust-toolchain.toml`・`package.json`・`.node-version` から読む。開発用 CI と同じ）。②`pnpm install --frozen-lockfile`。③`yunagi`（CLI）を `cargo build --release --locked -p yunagi-cli --target <target>` で作る。④`pnpm tauri build --ci --target <target> --bundles <bundles> --config src-tauri/release/common.json`（macOS・Windows は OS ごとの設定 `src-tauri/release/<os>.json` を重ねる）。⑤`scripts/release/package.ts` が配布物の名前と中身をそろえ、`.sha256` を添える（3.3）。⑥成果物（artifact `dist-<名前>`。14 日残す）として保存する。
- **配布用のビルドの設定は `tauri.conf.json` に入れず、`src-tauri/release/` の設定を `--config` で重ねる。** `tauri.conf.json` の `bundle.active` は `false` のままにする：`bundle.resources`・`bundle.macOS.files` の元のファイル（`THIRD_PARTY_LICENSES.md`・`cli-bin/yunagi`）は CI が置くもので、普段の `cargo test`・`pnpm tauri dev` にその有無を要求させないため。試験（`scripts/release/package.test.ts`）が、`tauri.conf.json` に配布用の設定が入らないことを確かめる。
- **Linux の arm64 だけ、arm のランナーが要る**（webkit2gtk の交差ビルドは難しいため）。`ubuntu-24.04-arm` は公開リポジトリでないと使えない（2026-11-05 時点）。リポジトリは 2026-11-06 に公開と決まったが（1章）、切り替え（YUN-109）までは非公開なので、まだ確かめられない。macOS の x86_64 は arm のランナー上で交差ビルドし、Windows の arm64 は x64 のランナー上で交差ビルドする。交差ビルドの組が実際に通るかは、試運転（下）と実機で確かめる。
- **試運転：** このファイル、ビルドの成否を決めるファイル（`src-tauri/` の `tauri.conf.json`・`Cargo.toml`・`capabilities/`・`icons/`・`build.rs`、直下の `Cargo.toml`・`package.json`・`rust-toolchain.toml`・`.node-version`・`vite.config.ts`、フォント）、`src-tauri/release/`、スクリプト、`about.toml`、許可リストの設定、ロックファイルを変える PR では、タグなしで同じ流れを走らせる（`release.yml` の `pull_request`）。リポジトリを公開するまで使えない `linux-arm64` を除く 5 組が動く（除外は `scripts/release/matrix.ts` の `DRY_RUN_SKIPPED` で決めている）。**ビルド（5 組。macOS・Windows の料金がかかる）は、リポジトリが公開のときだけ PR で自動で走る**（2026-11-05 決定。YUN-102。非公開の間は、料金が月の無料枠（12 ドル）を使い切って Linux のジョブも含めて全部止めるため。単価と見込みはテスト計画書 2章の「macOS・Windows での開発用 CI」）。非公開の間は、検査（`verify`。Linux だけ）が PR で走り、ビルドは手動で走らせる：`gh workflow run Release --ref <ブランチ>`（または Actions 画面の「Run workflow」）。手動もタグなしの試運転で、同じ 5 組が動く。公開した時点で、ワークフローを直さなくても、この 5 組は PR で自動で走り出す（`build` ジョブの `if` が `github.event.repository.private == false` を見るため）。`linux-arm64` は、公開しても試運転からは**除いたまま**で、自動では入らない（`DRY_RUN_SKIPPED` が決めている）。arm のランナーでの確認は、公開後に YUN-109 が行い、試運転に加えるかどうかも、その確認の結果で決める（YUN-109 の完了の条件に書いた）。タグの push の動きは変えない（常に全 6 組を走らせる）。**公開前（最初のタグを打つ前）に、手動で全部通すこと**（テスト計画書 6章の確認項目）。成果物は残るだけで、Release の作成・公開、署名鍵の使用は、タグの push でも試運転でもこの課題の範囲では行わない。許可外ライセンスの検査の失敗は、試運転でも落とす（3.1）。
- **キャッシュは使わない。** 開発用 CI の `Swatinem/rust-cache` や pnpm のキャッシュは、配布物を作る流れに、過去の実行が置いた物が混ざる経路になるため。ビルドの時間は長くなる。
- **Tauri が取得する道具は固定していない（既知の制限）。** Tauri のバンドラーは、Linux の AppImage の作成に `linuxdeploy`・AppRun・`linuxdeploy-plugin-appimage`（取得元の `continuous` は移動するリリース）、Windows の NSIS に NSIS 本体とプラグインを、ビルドの中でダウンロードする。action の SHA の固定（下の「action の固定」）の対象外で、Tauri の版を固定する以外の手段が今は無い。許容する理由：取得元は Tauri 公式の `binary-releases` と各プロジェクトの GitHub リリースで、ビルドの結果は 6 章の署名と `.sha256` で利用者が確かめる。署名鍵を持つ段階（YUN-101）では、取得物をハッシュで確かめる手順を足すかを決める（YUN-101 の完了の条件に書いた）。
- **ビルドの入力：** AppImage の作成は、FUSE の無いランナーでも動くよう `APPIMAGE_EXTRACT_AND_RUN=1` を付ける。

### 3.3 配布物の名前と中身

2026-11-05 決定（YUN-97）。2章の取り決めを、`scripts/release/package.ts` が作る（試験は `scripts/release/package.test.ts`）。

| OS | 配布物（出力先に置くもの） | 中身 |
| --- | --- | --- |
| macOS | `Yunagi_<ver>_<arch>.app.tar.gz` | ルートに `Yunagi.app`。CLI は `Yunagi.app/Contents/MacOS/yunagi`（`src-tauri/release/macos.json` の `bundle.macOS.files`）。`yunagi` が無ければ失敗する |
| Linux | `Yunagi_<ver>_<arch>.AppImage` | Tauri が作った AppImage の名前の付け替え |
| Linux | `yunagi-cli_<ver>_<arch>.tar.gz` | ルートの実行ファイル `yunagi` 1 つ |
| Windows | `Yunagi_<ver>_<arch>-setup.exe` | NSIS。`bin\yunagi.exe`（`src-tauri/release/windows.json` の `bundle.resources`）と、アプリ本体の `Yunagi.exe`（同 `mainBinaryName`）を含む |

- 配布物ごとに `<ファイル名>.sha256`（`<64 桁の 16 進>  <ファイル名>` の 1 行。2章）を作る。
- Tauri がビルドで付ける名前（Linux の `amd64`・`aarch64`、Windows の `x64`・`arm64` など）は、出力から配布物を探して `<arch>` を `x86_64`・`arm64` に付け替える（出力先に該当のファイルがちょうど 1 つでなければ失敗する）。
- macOS の本体の実行ファイルは `Contents/MacOS/yunagi-app`（Tauri の既定）のままにする：`mainBinaryName` を `Yunagi` にすると、`Contents/MacOS/Yunagi` と CLI の `Contents/MacOS/yunagi` が、大文字小文字を区別しない既定のファイルシステムで同じ名前になってしまうため。Windows はディレクトリが違う（本体はインストール先、CLI は `bin\`）ので `Yunagi.exe` にできる。`yunagi show` の探し方（`crates/cli/src/show.rs`）はどちらにも合っている。
- macOS の `.app.tar.gz` は、いまは `tar` で自分たちが作る。Updater 用の署名 `.sig` を付ける段階（YUN-101）で、Tauri の `createUpdaterArtifacts` が作るものに置き換えるか、`tauri signer sign` でこの tar.gz に署名するかを決める。アドホック署名（3 の 4）は、`.app.tar.gz` を作る前と、`.sig` を付ける前に済ませる（署名が変わると `.sig` が合わなくなるため）。
- 成果物（`actions/upload-artifact`）は実行権限を残さないので、成果物から取り出した AppImage は実行できない（`.tar.gz` は権限を保つ）。Release に添付する段階（YUN-101）で、`chmod +x` するか、AppImage を権限ごと包んで渡すかを決める。利用者は `install.sh` が権限を付けて置くので影響しない。
- Linux の `.tar.gz`（アプリ全体）と `.deb`（2章で任意）は、置き場所（4.1 の末尾）を決めていないので、まだ作らない。決めるときに足す。
- `bundle.createUpdaterArtifacts` はまだ有効にしない：署名の秘密鍵が無いとビルドが失敗するため（5.2 の②は、鍵ができてから。YUN-100・YUN-101）。
- 実機・実際のリリースでしか確かめられない点（NSIS の中身に `bin\yunagi.exe` と `Yunagi.exe` があること、各 OS の配布物の起動、交差ビルドの組、arm の Linux のビルド）は、08 の 4.9 に足した。

### 3.4 署名と公開（YUN-101 で作る）

3 の 4・5・6（`SHA256SUMS` 以降）・7 は、秘密鍵（YUN-100）と、リポジトリの公開（YUN-109）・公式サイトの公開（YUN-107）が済んでから動かせる（置き場所は決まっている：1章・2章）ので、子課題 YUN-101 が作る。YUN-97 では作らず、動かしてもいない。作るもの：

- macOS のアドホック署名（3 の 4）。
- Updater 用の署名 `.sig`（`bundle.createUpdaterArtifacts`。5.1 の「署名に版を入れる」）と、`latest.json`（5.1。トップレベルの `version` を入れ子の項目（`platforms` など）より前に置く：4.1 の 2）。
- 全配布物の `SHA256SUMS` と、配布用鍵での `SHA256SUMS.sig`（2章・6.2）。
- **公式サイトの公開（Pages へのデプロイ）の段**（2026-11-06 決定。YUN-103）：リリースのワークフローの最後の段として、GitHub Release の配布物が `<配布元>` から取得できることを確かめた**あとだけ**、サイトを `latest.json` つきでデプロイする（確認が通らなければデプロイしない。先に `latest.json` だけが公開される順序の入れ替わりを防ぐ）。サイトだけの変更のデプロイ（YUN-107）は、**いま公開中の `latest.json` を取得して含める**（取得できなければ中止する。含め忘れて 404 にしない、古い写しで上書きしない）。取り下げて戻したあとも、この取得のおかげで戻した状態が引き継がれる。どちらの段も 2章の secrets の決まり（フォークの PR に渡さない・Environment で制限する）に従う。
- GitHub Release の作成と、配布物・`.sha256`・`.sig`・`SHA256SUMS`・`SHA256SUMS.sig`・`THIRD_PARTY_LICENSES.md` の添付。
- Homebrew tap への自動 PR（3 の 7。4.3）。
- 「署名鍵を持つジョブの権限」：鍵を使うジョブだけに `contents: write` と鍵のシークレットを渡し、検査・ビルドのジョブには渡さない（このファイルの `permissions` は `contents: read`）。

### action の固定

開発用 CI（`.github/workflows/ci.yml`）と配布用 CI のどちらも、ワークフローが使う action を次のとおり固定する（2026-11-05 決定。YUN-43）。タグは後から別のコミットに付け替えられ、乗っ取られた action が CI で動くおそれがある（配布用 CI ではビルドの署名鍵の近くで動く）ため、中身が変わらないコミットで指定する。

- `uses:` に書く action は、すべて 40 桁のコミットの SHA で指定し、同じ行の末尾に版のタグをコメントで併記する（例：`uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1`）。`actions/*` の公式の action も例外にしない。ブランチ名（`main`）、移動するタグ（`v7` のようなメジャー版だけのタグ）では指定しない。コメントの版は、`v7.0.1` のように完全な版で書く。
- 同じリポジトリの中の再利用ワークフロー・composite action（`./.github/...`）は、同じコミットのものを使うので、固定の対象外にする。Docker の image を使う `docker://` は、使うなら `sha256:` のダイジェストで指定する（今は使っていない）。
- SHA は、action のリポジトリに実在するタグから解決する（`gh api repos/<owner>/<repo>/git/ref/tags/<タグ>` で得た `object.sha` が、注釈付きタグ（`type` が `tag`）なら `gh api repos/<owner>/<repo>/git/tags/<その sha>` で、指す先のコミットの SHA を得る）。コメントの版と SHA が食い違わないよう、記憶や別の場所からの写しで書かない。書いた SHA は、`gh api repos/<owner>/<repo>/commits/<タグ> -q .sha` の結果と一致するか照合する（フォークだけにあるコミットの SHA を書かないため。設定の「SHA での固定を必須にする」は SHA の長さしか見ない）。
- 更新は Dependabot（`.github/dependabot.yml` の `github-actions`）に任せる。週に 1 回、新しい版があれば PR を出し、SHA とコメントの版を一緒に書き換える。公開から 14 日たった版だけを PR にする（`cooldown`。SECURITY.md 4.4 の依存の更新と同じ目安）。脆弱性の修正の PR は、この待ち時間に関係なく出る。人が手で足す・上げるときも、公開から 14 日を経た版を採る。Dependabot の PR は、minor・patch の更新を 1 つにまとめて出し（`groups`）、major の更新は action ごとの別の PR にする。オーナーが、リリースの変更点（release notes）と差分を見て、CI が通っていることを確かめてからマージする。CI が失敗した PR はマージしない。原因が 1 つの action なら、その更新だけを `dependabot.yml` の `ignore` に載せて（理由をコメントに書く）PR を作り直させる。直すのに作業が要るときは、通常の課題にして進める。Dependabot の PR は、Linear の課題・ブランチ名（`yun-<番号>-...`）・`/code-review` の決まりの例外で、タイトルもブランチ名も既定のままにし、`/code-review` は行わない（CLAUDE.md の「ブランチと PR」にも書いた）。
- 固定していない action が入るのを防ぐため、リポジトリの設定（Settings → Actions → General）で「Require actions to be pinned to a full-length commit SHA」を有効にする。オーナーの設定の作業で、Linear の YUN-98 が追う。有効にするのは、この固定の PR（YUN-43）をマージし、ほかの開いている PR（Dependabot の PR も含む）も main に追いつかせるか閉じたあと（有効にすると、タグ指定のワークフローは実行されずに失敗するため）。有効にした後で「not pinned」のような失敗が出たら、その PR がまだタグ指定のままなので、main に追いつかせる（Dependabot の PR は `@dependabot rebase`）。配布用 CI を作るときも、この決まりに従う（YUN-97 の完了の条件に書いた）。
- Dependabot の対象は action だけで、Rust・JavaScript の依存の更新は含めない（SECURITY.md 4.4 のとおり、14 日の目安で別に更新する）。

## 4. インストールスクリプト

### 4.1 macOS・Linux（install.sh）

```sh
curl -fsSL https://getyunagi.com/install.sh | sh
```

スクリプトの動作：

1. OS とアーキテクチャを判定する。未対応の組み合わせは明確なメッセージで終了する。
2. 最新版（または `YUNAGI_VERSION` で指定した版）の配布物と `.sha256` を取得する。最新版は、GitHub の「latest」ではなく `https://getyunagi.com/latest.json` の `version` で決める（スクリプトは、ファイルの中で最初に現れる `"version"` の値を読む。そのため `latest.json` は、トップレベルの `version` を、入れ子の項目（`platforms` など）より前に置く。Tauri Updater の形式でも守れる。配布用 CI（YUN-97）の条件。取り下げ時に `latest.json` を前の版に戻すと、新しく入れる利用者も前の版になるため。2章）。スクリプトが使う基本の道具（`curl`・`tar`・`awk`・`grep`・`sed`・`head`・`mktemp`・`cp`・`mv`・`ln` など）が無ければ、最初に名前を挙げて止まる。取得元は環境変数で変えられる：`YUNAGI_VERSION`（`v` が付いていても付いていなくてもよい）、`YUNAGI_BASE_URL`（配布物の取得元。既定は 2章の `<配布元>`。末尾の `/` は取り除く）、`YUNAGI_LATEST_URL`（`latest.json` の場所）。`YUNAGI_BASE_URL`・`YUNAGI_LATEST_URL` は、試験で偽の配布元に向けるためにも使う（4.5）。スクリプトの既定値は、`install/install.sh`・`install/install.ps1` の先頭の 2 か所に実際の値で書いてある（配布元は `https://github.com/misty-swan/yunagi/releases/download`、`latest.json` は `https://getyunagi.com/latest.json`。YUN-11）。仮の表記のときに「変数を設定するよう案内して終わる」動きは、既定値が決まったので無くした。試験は、既定の場所を実際には取りに行かず（本物のネットワークに出ない）、スクリプトに書いてある値だけを確かめ、動作は偽の配布元（`YUNAGI_BASE_URL`・`YUNAGI_LATEST_URL`）に向けて確かめる。
3. ハッシュを照合し、一致しなければ中止する（何も置かない。配布物ごとに、取得した直後に照合する）。
4. macOS：`~/Applications/Yunagi.app` に展開する（`/Applications` は権限が必要なため既定にしない）。Linux：`~/.local/opt/yunagi/Yunagi.AppImage` に AppImage を、`~/.local/opt/yunagi/bin/yunagi` に CLI を置き、デスクトップエントリを `~/.local/share/applications/io.github.misty-swan.yunagi.desktop` に作成する（ファイル名はアプリの識別子にそろえる。アーキテクチャ設計書 6.1）。アプリ本体の配置先は、データディレクトリ（アーキテクチャ設計書 6章。Linux は `~/.local/share/yunagi/`）とは別にする。
   - Linux は AppImage と、CLI だけの配布物 `yunagi-cli_<ver>_<arch>.tar.gz`（2章）の 2 つを取って照合する。CLI の配布物は、中身を確かめてから置く（`yunagi` が無ければ何も置かずに止まる）。置くときは、AppImage と CLI の両方を置き場所の隣の一時ファイルにコピーし終えてから、両方の名前を付け替える（動いているアプリを途中の状態で上書きしない。コピーの途中の失敗（容量不足など）で、新旧が混ざった状態にならない。失敗したときの一時ファイルは終了時に消す）。AppImage の起動には FUSE（`fusermount`）が要るので、見つからなければ、置き終えた後に、FUSE 2 のパッケージを入れるか `--appimage-extract-and-run` で起動するよう知らせる（導入は止めない）。
   - デスクトップエントリは、`$XDG_DATA_HOME/applications/`（`XDG_DATA_HOME` が無い・絶対パスでないときは `~/.local/share/applications/`。データディレクトリと同じ決め方）に作る。`Exec` は AppImage の絶対パスを `"` で囲んで書く。アイコンは付けない（AppImage の中のアイコンは取り出さない。見た目は実機で見る。08 4.9）。置き場所（`$HOME`）に `"`・`\`・`$`・`` ` ``・`%` のどれかが含まれるときは、`Exec` の書き方が起動側ごとに違うので、デスクトップエントリを作らず、そう知らせる（ほかは置く）。
   - macOS は、`~/Applications/Yunagi.app` が既にあれば `Yunagi.app.old` に退けてから新しいものを置き、置けなければ元に戻して止まる（動いているアプリや容量不足で置換に失敗しても、動いていたものを失わない）。展開した中に `Contents/MacOS/yunagi` が無ければ、何も置かずに止まる。
   - macOS で `uname -m` が `x86_64` でも、`sysctl -n hw.optional.arm64` が `1` なら arm64 の Mac（Rosetta の下のシェル）なので、arm64 の配布物を取る。
   - CLI の配布物は全体を一時フォルダに展開し、`yunagi` を探す（メンバー名が `yunagi` でも `./yunagi` でもよい）。
5. CLI を `~/.local/bin/yunagi` にリンクし（macOS は `~/Applications/Yunagi.app/Contents/MacOS/yunagi` へ、Linux は `~/.local/opt/yunagi/bin/yunagi` へ）、PATH に含まれていなければ追記方法を案内する。`~/.local/bin/yunagi` が、リンクでない普通のファイルとして既にあるときは、利用者のものかもしれないので上書きせず、そう伝えて止まる（リンクなら付け替える）。
6. 何をどこに置いたかを最後に一覧表示する。
7. Linux で Wayland のセッション（環境変数 `XDG_SESSION_TYPE` が `wayland` のとき、または `x11` 以外で `WAYLAND_DISPLAY` があるとき。操作仕様書 1章と同じ判定）のときは、最後に次の案内も表示する（Wayland では、OS 全体のショートカット Mod+Shift+D が効かないことがあるため。操作仕様書 1章「OS 全体の操作」、アーキテクチャ設計書 3.5）。デスクトップ環境のキーボード設定で、コマンド `~/.local/bin/yunagi show`（PATH に含まれていれば `yunagi show`）を好きなキーに割り当てる。設定の場所は、GNOME は「設定」→「キーボード」→「キーボードショートカット」→「カスタムショートカット」、KDE Plasma は「システム設定」→「キーボード」→「ショートカット」→「カスタムショートカットを追加」、Hyprland・Sway などのタイル型は設定ファイルのキー割当（`bind`）。同じ手順を利用者ガイド 2章にも載せる。案内には、`--note` の別のキー（ノートモードで開く `yunagi show --note`）と、ふだんのシェルの `PATH` を引き継がないキー設定では、フルパス `~/.local/bin/yunagi show`（展開した `$HOME` の実際のパス）で書く必要があることも添える（2026-11-04 決定。YUN-88）。`yunagi show`（アプリを前面に出す CLI。MCP・CLI 仕様書 3.3）は作ってあり、案内は設定の Wayland の文（操作仕様書 1章「OS 全体の操作」。文言は `src/lib/i18n_settings.ts` の `operation.global.wayland`）と食い違わせない（コマンド名は `yunagi show`）。案内は `install/install.sh` の `wayland_note` が出す（2026-11-05 決定。YUN-95）。英語で書く（設定の英語の文と同じ「assign the command yunagi show to a key of your choice」を含め、端末の文字コードに左右されないため）。コマンドは、`~/.local/bin` が `PATH` に入っていなければフルパス（展開した `$HOME` の実際のパス）で、入っていれば `yunagi show` と書く。
   - スクリプトの表示は、すべて英語にする（CLI と同じ。端末の文字コードに左右されない）。
   - `yunagi show` は、実行ファイルの実体（リンクを解いたもの）からアプリを探す。`~/.local/bin/yunagi` はリンクなので、`crates/cli/src/show.rs` は `current_exe` の結果を `canonicalize` してから `locate` に渡す（macOS の `current_exe` はリンクのままのパスを返すことがある。Linux は `/proc/self/exe` で実体が返る）。
   - `yunagi show` がアプリを探す場所（MCP・CLI 仕様書 3.3）は、この章の配置に合わせている：Linux は `~/.local/opt/yunagi/bin/yunagi` の隣の `../Yunagi.AppImage`、macOS は `yunagi` を含む `Yunagi.app`、Windows は `bin\yunagi.exe` の隣の `..\Yunagi.exe`。配置をこれと変えるときは、探す場所（`crates/cli/src/show.rs` の `locate`）も変える。tar.gz・.deb の置き場所は、まだ決めていない（決めるときに足す）。

スクリプトは `sh` 互換（POSIX sh。`[[ ]]`・`pipefail`・`local` は使わない）で書き、`set -eu` で途中失敗時に止める。スクリプト自体も公開リポジトリで公開し、内容を確認してから実行する方法（ダウンロードして読む手順）を導入ガイドに載せる。置き場所は、リポジトリ直下の `install/install.sh`（2026-11-05 決定。YUN-95）。`curl | sh` はダウンロードが途中で切れても、そこまでを実行してしまう。そのため、本体を関数 `main` に包み、最後の行で呼ぶ（全体が届かないと何も動かない）。一時フォルダは、終了時に必ず消す。

### 4.2 Windows（install.ps1）

```powershell
irm https://getyunagi.com/install.ps1 | iex
```

動作は install.sh に準じる（環境変数も同じ。4.1 の 2）。`Yunagi_<ver>_<arch>-setup.exe` を取得してハッシュを照合し、`/S`（サイレント）と `/D=%LOCALAPPDATA%\Programs\Yunagi`（インストール先。NSIS の決まりで最後の引数にし、引用符を付けない）で実行する（2026-11-05 決定。YUN-95。インストーラーの既定の場所に頼らず、スクリプトが指定する）。Yunagi のアプリ（`%LOCALAPPDATA%\Programs\Yunagi\Yunagi.exe`。CLI の `yunagi.exe` は数えない。MCP サーバーなどが動いていても止めないため）が動いているときは、インストーラーを動かす前に、終了してから実行し直すよう知らせて止まる。終了コードが 0 でなければ止まり、`%LOCALAPPDATA%\Programs\Yunagi\bin\yunagi.exe` があることを確かめてから、ユーザー環境変数の PATH に `bin` を追加する（レジストリの `HKCU\Environment\Path` を、`%VAR%` を展開せずに読み、`REG_EXPAND_SZ` のまま書く。`[Environment]::SetEnvironmentVariable` は使わない。展開した文字列で上書きして、利用者の `%JAVA_HOME%\bin` のような項目を固定してしまうため。レジストリを直接書くと実行中のプログラムに変更が伝わらないので、使い捨てのユーザー変数を .NET で設定して消し、環境の変更の通知（`WM_SETTINGCHANGE`）を送る）（すでにあれば足さない。`%VAR%` を展開した形が同じ場所のときも足さない。空の項目は捨てる。大文字小文字と末尾の `\` の違いは同じとみなす）。同じ端末では、現在のセッションの `PATH` にも足し、すぐ `yunagi` を使えるようにする。スタートメニューのショートカットは、インストーラー（NSIS）が作る（スクリプトは作らない）。インストーラーは `bin\yunagi.exe` を含める（2章。配布用 CI の側の条件）。失敗は例外（エラーの表示）で知らせる。`irm | iex` は終了コードを返さないので、自動化で失敗を見分けたいときは、スクリプトを保存して実行する（導入ガイドの「内容を確認してから実行する」手順）。表示は install.sh と同じく英語。スクリプトは ASCII だけで書く（Windows PowerShell 5.1 は、BOM の無いファイルを ANSI で読むため）。

### 4.3 Homebrew（macOS）

```sh
brew install <tap名>/yunagi
```

tap 内の Cask でアプリと CLI を配置する。アプリは Cask の既定どおり `/Applications/Yunagi.app` に置き、CLI は Cask の `binary` で Homebrew の bin にリンクする。

**アンインストール（`zap`）**（2026-11-05 決定。YUN-44）：Cask は `uninstall` でアプリと CLI のリンクを外し（Cask の既定）、`zap` で `yunagi uninstall --purge`（4.4）が消す macOS の場所を消す（違いは下の段落）。`brew uninstall --cask --zap yunagi` で、アプリとデータをまとめて消せる。`zap` の内容は、アーキテクチャ設計書 6.5 の macOS の場所と、データディレクトリ（6章）である：

```ruby
uninstall quit: "io.github.misty-swan.yunagi"

zap delete: [
  "~/Library/Application Support/Yunagi",
  "~/Library/Caches/io.github.misty-swan.yunagi",
  "~/Library/HTTPStorages/io.github.misty-swan.yunagi",
  "~/Library/Preferences/io.github.misty-swan.yunagi.plist",
  "~/Library/Saved Application State/io.github.misty-swan.yunagi.savedState",
  "~/Library/WebKit/io.github.misty-swan.yunagi",
]
```

`zap` は `trash:`（ゴミ箱へ移す）でなく `delete:`（すぐ消す）にする。ゴミ箱に残ると、データを消す約束にならないため。`uninstall quit:` は、動いているアプリを先に終了させる（消した後に WebView や設定の保存が、ファイルを書き戻さないため）。`delete:` は無いものを飛ばすので、6.5 で実機が未確認の場所を含めても害はない。`yunagi uninstall --purge` との違い：①アプリの終了を頼む（`yunagi` は動いていれば止まる）。②`~/Library/Application Support/Yunagi` を、保存先を変えているかにかかわらず丸ごと消す（`--purge` は既定ディレクトリを全部消すので、既定のままなら同じ。保存先を変えているときは、`zap` は変えた先に触れない）。③plist は `defaults delete` を通さず直接消すので、OS の設定の仕組みが書き戻して残ることがある（アーキテクチャ設計書 6.5。残ったら手で消す。plist は設定の値だけで、ノートのデータではない）。④`config.json` を読まず、順番も取らない。識別子（アーキテクチャ設計書 6.1）か 6.5 の場所を変えたら、この一覧も同時に直す。`zap` は `config.json` を読めないので、設定で変えた保存先は消えない（利用者ガイドの「アンインストール」に書く）。tap のリポジトリはまだ無い。作るときにこの内容で書く。

Cask はダウンロードしたアプリに隔離属性（`com.apple.quarantine`）を付けるため、アドホック署名のままでは初回起動が Gatekeeper に止められる見込みである（要確認：Homebrew の挙動は変わりうるため、実機で確認する）。Cask の `postflight` で隔離属性を外す。外せない場合は、導入ガイドに対処（`xattr -dr com.apple.quarantine /Applications/Yunagi.app`）を載せる。

### 4.4 アンインストール

`yunagi uninstall` で、アプリ本体と CLI のリンク、Linux ではデスクトップエントリ（4.1）を削除する。データディレクトリは既定では残し、`--purge` 指定時のみ、データと WebView のデータ（アーキテクチャ設計書 6.5。置き場所と消す決まり）を削除する。アプリが動いているときは、`--purge` の有無にかかわらず何も消さずに止まる（下の順番の 1）。消すものは、インストールスクリプトが最後に一覧表示するもの（4.1 の 6、4.2）と一致させる：macOS は `~/Applications/Yunagi.app` と `~/.local/bin/yunagi`、Linux は `~/.local/opt/yunagi/`・`~/.local/bin/yunagi`・デスクトップエントリ、Windows は Yunagi 自身のアンインストーラー（インストール先とスタートメニューのショートカットを消す）と、ユーザーの PATH に足した `bin`。Linux のデスクトップエントリは `$XDG_DATA_HOME/applications/io.github.misty-swan.yunagi.desktop`（4.1 の 4 と同じ決め方）。macOS の `Yunagi.app.old`（置換の途中で退けるもの）は、置換が終わると消えるので、消す対象に数えない。

どの導入かは、実行している `yunagi` の実体（リンクを解いたパス）で見分ける（2026-11-05 決定。YUN-44）。見分けは、下の順番の前に行う（Homebrew の人に「アプリを終了して」でなく、先に `brew` を案内するため）。

- 正規の置き場所（4.1・4.2 が置く場所）：macOS は `~/Applications/Yunagi.app/` の中、Linux は `~/.local/opt/yunagi/` の中、Windows は `%LOCALAPPDATA%\Programs\Yunagi\` の中。この中なら、上の一覧を消す。
- Homebrew の Cask：実体が `/Applications/Yunagi.app/` の中にあり、かつ Caskroom の `yunagi`（`$HOMEBREW_PREFIX/Caskroom/yunagi`。`HOMEBREW_PREFIX` が無ければ `/opt/homebrew`・`/usr/local` の両方）が在るとき。`--appdir` で別の場所に入れた Cask は見分けられず、次の「どちらでもない」になるので、その案内に「Homebrew で入れたなら `brew uninstall --cask yunagi` を使う」を添える（Cask の `binary` の行き先は `/Applications/Yunagi.app/` の中になるので、パスだけでは手で `/Applications` に置いたものと見分けられない）。何も消さずに、`brew uninstall --cask yunagi`（`--purge` のときは `--zap` 付き）を案内して止まる。Cask が置いたものを `yunagi` が消すと、Homebrew の記録と食い違うため。
- どちらでもないとき（手で `/Applications` に置いたもの、開発用のビルド、置き場所を決めていない tar.gz・.deb など）：見つけたパスを示し、何も消さずに止まる。

消す順番（動いている `yunagi.exe` はアンインストーラーが消す `bin\` の中にあり、ロックファイルは消すデータの中にあるため、Windows で失敗しない順にする。macOS・Linux も同じ順にそろえる）：

0. 見分けの後、ロックを取る前に、確認できる端末かを見る。端末でなければ、何も消さずに `permission_denied`（終了コード 3）で止まる（05 3.1。エージェントもシェルから呼べるため）。
1. `yunagi.lock`（アーキテクチャ設計書 4.2）を取る。既定ディレクトリが無いとき（入れただけで一度も起動していない）は、ロックも `config.json` の読み込みも飛ばして、2 へ進む（4.2：ロックの関数はフォルダを作らない）。取れなければ（直接モードと同じく 5 秒まで待つ）、アプリかエージェントの MCP サーバーが動いているので、何も消さずに、それらを終了するよう知らせて止まる（`busy`、終了コード 7）。`--purge` なら、続けて `config.json` を読み、保存先を求める。読めない・壊れているときは、何も消さずにエラーで止まる（保存先が分からないまま既定ディレクトリを消すと、保存先のデータが `config.json` という手がかりごと残って、消したことにならないため。`storage_unavailable`、終了コード 6）。`--purge` が無いときは `config.json` を読まない。続けて、いまある消すものを一覧にして `[y/N]` の確認を得る（取りやめは何も消さずに終了コード 0）。確認の質問に答える間も、ロックは持ったままにする（答えを待つ間にアプリが起動して、消している最中のデータを掴まないため。答えないで放っておくと、その間はアプリが起動できない）。既定ディレクトリがシンボリックリンクのときは、リンクだけを消し、先のフォルダは消し残りとして一覧に出す（先には触れない決まり。アーキテクチャ設計書 6.5）。消すものが何も無ければ、確認せずに終わる。
2. `--purge` なら、WebView のデータと、保存先の `yunagi.db` などと、既定ディレクトリの中の `yunagi.lock` 以外を消す。macOS の plist は、消す前に `defaults delete` を呼び、消した後にまた現れていたら消し残りに数える（アーキテクチャ設計書 6.5）。
3. アプリ本体と CLI のリンクを消す。`~/.local/bin/yunagi` は、この導入を指すシンボリックリンクのときだけ消す（先が正規の置き場所の中。`install.sh` も、リンクでないファイルは置き換えないため）。手で置いたファイルなどは触れず、消し残りとして一覧に出す。Windows は、2 より先に後片付けを始める（`yunagi.exe` の終了を待つので、先に始めても消し始めない。始められなければ、データも `PATH` も何も変えずに止まる。アプリが残ったままデータだけ消えるのを避けるため）。始められたら 2 を行い、最後にユーザーの `PATH` から `bin` を外す（インストールスクリプトが足した項目。`/` と `\` は同じとみなす。大文字小文字・末尾の `\`・`%VAR%` を展開した形の違いは同じとみなし、ほかの項目は書かれたまま残す。`REG_EXPAND_SZ` のまま書き、環境の変更の通知を送る。4.2 と同じ）。NSIS のアンインストーラーは `yunagi.exe` から切り離して起動する。切り離す後片付けは `powershell.exe`（4.2 のスクリプトと同じく、Windows 10 以降に入っている 5.1 で動く文にする）で、`yunagi.exe` の終了を待ってから `uninstall.exe /S _?=<インストール先>`（`_?=` は、アンインストーラーを自分のコピーに移さず、その場で動かして終わりを待たせる指定。NSIS の決まりで最後の引数に置く）を動かし、終わったら、インストール先の残り（`uninstall.exe` 自身など）と、`--purge` なら既定ディレクトリ（残りの `yunagi.lock` を含む）を消す。`yunagi.exe` は終わりを待たずに終了する。`uninstall.exe` が無いときは、その旨を知らせに書き（消し残りには数えない）、インストール先は同じ後片付けで消す。後片付けの失敗は、`yunagi.exe` が終わった後なので報告できない（エラーは無視して続ける）。知らせの文で「あとで残っていたら手で消す」と伝えるだけにする（ウイルス対策ソフトなどがファイルを掴んでいるときの消し残り）。既定ディレクトリがシンボリックリンクのときは、後片付けには渡さず（`Remove-Item -Recurse` が先に触れないように）、リンクだけをこちらで消す。
4. macOS・Linux は、`--purge` なら最後に `yunagi.lock` と既定ディレクトリを消す（開いたまま消せる）。ほかが消せずに既定ディレクトリが空にならなかったときは、そのフォルダは残し、消せなかったものとして挙がっている。

ロックは最後の削除まで離さない。`yunagi.lock` を消すのは、「ロックファイルは消さない」（アーキテクチャ設計書 4.2）の例外で、`--purge` の最後の 1 回だけ。消す前のファイルを掴んだ待ち手と、作り直されたファイルを掴んだ新しい起動が両方ロックを取れてしまう危険は、消している最中にアプリや MCP サーバーを起動しない使い方に頼って受け入れる（4.2 にも書く）。Windows でプロセスが終了してから後片付けが消すまでの短い間だけは、ロックが無いが、その間にアプリや MCP サーバーが起動する使い方は想定しない（起動すると、消し残りが出るので、知らせの文に「終了後に残っていたら手で消す」と書く）。

**終わりの表示と終了コード**（2026-11-05 決定。YUN-99）：消したもの・消せなかったもの（理由つき）を標準出力に一覧にする。消せなかったものがあっても、ほかは消し続ける。消せなかったものが無ければ 0、あれば 1。消す場所は上に書いた完全なパスだけで、シンボリックリンクはリンクだけを消す（アーキテクチャ設計書 6.5）。Windows の後片付けは終了後に動くので、その旨と対象のパスを知らせ、「あとで残っていたら手で消す」と書く。

**試験の作り**（コードは `crates/cli/src/uninstall/`）：場所は OS の区分と環境変数の組から求める関数（試験は Linux で macOS・Windows の場合も確かめる）。消すものはすべて 1 つの層を通り、通す前に「絶対パスで、`..` を含まず、ルートや利用者のホームそのものではない」ことを確かめる。試験では、一時フォルダの外を消そうとしたら消さずに panic する決まりを加える（利用者の実際の環境を消さないため。結合テストは `HOME`・`XDG_DATA_HOME` を一時フォルダに差し替え、動かす前に差し替えが効いていることを表明する）。実機でしか確かめられない部分（`defaults delete`、Windows の `PATH` の読み書き、切り離した後片付けの起動）は、呼び出しの口を差し替えられるようにして偽物で確かめる。実機の項目はテスト計画書 4.9。

置く物を変えたら、ここと 4.1・4.2・4.3 の `zap` を同時に直す。2026-11-05 に、この一覧が `install/install.sh`・`install/install.ps1` の置くものと一致していることを確かめた。

### 4.5 インストールスクリプトの試験

2026-11-05 決定（YUN-95）。試験は `install/install.test.ts`（vitest。`pnpm check` の `check:web` に含まれ、開発用 CI でも走る）。利用者の環境には触れない：`HOME`・`XDG_*` は一時フォルダにし、`uname` は偽物に差し替え、配布物は一時フォルダに作った偽の配布元から `file://` で取る。

- `install.sh`：OS・アーキテクチャの組み合わせ（Linux x86_64・arm64、macOS arm64・x86_64、未対応の OS・アーキテクチャ）、配置先とリンク、デスクトップエントリ、置き換え、ハッシュ不一致・`.sha256` の形の誤り・配布物が無い・CLI が入っていないときに何も置かないこと、版の指定（`v` 付き・形の誤り・`latest.json`）、`curl | sh` と同じ標準入力からの実行、PATH の案内、Wayland の案内（判定 3 通りと内容。設定の英語の文との食い違い）。
- `install.ps1`：PowerShell（`pwsh`）があるときだけ走る（無ければ飛ばす。GitHub の ubuntu-24.04 には入っている）。構文、ASCII だけであること、アーキテクチャの読み替え、版の形、`.sha256` の読み方、ハッシュの照合、PATH への追加の判定。インストーラーを動かす部分は、Windows でしか動かないので、実機で確かめる（テスト計画書 4.9）。
- 飛ばされた試験は成功とは別なので、`pwsh` のある CI でも走ることを、CI のログで確かめる。

## 5. 自動更新

- 更新の自動確認は既定で有効とし、設定（一般 →「更新の自動確認」）で無効にできる。確認時に送る情報は版・OS・アーキテクチャのみ（配布ホストには IP アドレスとアクセス日時が記録される）。
- 有効な場合、アプリ起動時と以後24時間ごとに `latest.json` を確認する。
- 新しい版があれば、サイドバー下端の「New version」で控えめに通知し（画面設計書 3章）、利用者の操作（「New version」を選んで「更新して再起動」、またはパレットの「更新して再起動」コマンド）で更新する（作業中に勝手に再起動しない）。
- 更新前にデータベースのバックアップを作成する。マイグレーションが失敗した場合は自動でバックアップに戻してデータを変更せず、エラー画面で旧版の再導入方法（`YUNAGI_VERSION=<旧版>` を付けてインストールスクリプトを実行）を案内する。データベースが新しい版で作られていて開けない場合（旧版を入れ直したときなど）は、新しい版の入れ方（`YUNAGI_VERSION` を付けずに実行）を案内する（画面は画面設計書 10.1）。
- 未決：Homebrew の Cask で入れたアプリ（4.3。`/Applications/Yunagi.app`）の更新。アプリ内の更新で入れ替えると、Homebrew の記録の版と食い違う。アプリ内の更新を出さず `brew upgrade --cask yunagi` に任せるか、アプリ内の更新を使うか（Cask の `auto_updates true`）を、tap を作るとき（YUN-101）に決める。決まるまで、利用者ガイド 1 章「更新」には「Homebrew で入れた場合の更新の手順は決めて書く（要確認）」の 1 行を置いてある。決めたら、その行を手順に置き換える。

### 5.1 動きの細部

2026-11-05 決定（YUN-83）。実装は `src-tauri/src/updater.rs`（Tauri のプラグイン `tauri-plugin-updater` 2.14.0 の Rust の API）。

- **確認は Rust 側だけで行う。** 画面（WebView）には、状態を読む `get_update_state`、イベント `update-state`、`install_update` だけを見せ、プラグインの JavaScript の API と権限（`capabilities/default.json` の `updater:*`）は渡さない。画面から配布元や公開鍵を指定できないようにするため。通信は Rust 側なので、画面の CSP（`connect-src`）は変えない。
- **通信するのは、次のすべてを満たすときだけ。** ①自動更新の対象の配布である（開発ビルドは対象外。Linux は AppImage だけで、`.deb`・`.tar.gz` は対象外なので確認も通知も出さず、再導入で更新する。2章）。②公開鍵と配布元が設定されている（5.2）。③設定 `update_check` が有効である。③は確認のたびにデータベースから読み直し、覚えておかない。データベースを開く前（起動時の画面の間）は読めないので確認せず、5 秒ごとに読み直して、読めた後に初めて確認する（既定は有効なので、読めないうちに確認すると、無効にしている利用者にも通信が出てしまうため）。無効なら、起動時も 24 時間ごとの時刻が来ても通信しない。
- **設定を変えたとき**は、すぐ確かめ直す。有効にしたら直ちに確認し、無効にしたら表示している「New version」を消す（確認中に無効にされたら、結果は使わない）。
- **確認の頻度**は、データベースを開いて設定を読めた直後に 1 回、以後 24 時間ごと。手動で確認する操作は設けない（通知の仕組みを 1 つに保つため）。
- **確認の失敗は画面に出さない。** ログに残し、次の確認（24 時間後）でまた試す。通知を出している間に確認が失敗しても、通知は消さない。配布元が新しい版を引っ込めたとき（取り下げ）は、次の確認で通知を消す。ダウンロードに失敗した版が見つかり続けても、失敗の表示は消さない（別の版が見つかる・取り下げられる・選び直すまで残す）。確認の通信は 30 秒、ダウンロードの通信は 10 分で時間切れにする（止まった通信で確認のスレッドが塞がらないように）。ダウンロード中・再起動中は確認しない（入れ替える版を動かさないため）。
- **更新内容**（`latest.json` の `notes`）は配布元の文なので、信用しない文字列として扱う。プレーンテキストで出し（HTML・Markdown として解釈しない）、2000 字で切る。
- **「更新して再起動」の順序：** ①画面が入力中の内容を保存し終える。保存に失敗している間は始めない（画面に理由を出す）。②ダウンロードして署名を検証する（失敗したら「更新できませんでした」を出して終わる。何も入れ替えていないので、そのまま使えて、選び直せる）。③データベースを閉じる（「再起動します」を画面に知らせてから 1.5 秒待つ。画面はその間、入力を受け付けず、入力中の内容を保存し終える。閉じた後に打たれた文字が消えないように）。④入れ替える。ダウンロードは、画面に出していた版を頼む（その間に確認が別の版を見つけていたら失敗にする）。⑤再起動する。③の後は画面のコマンドが `not_ready` になるので、入れ替えが失敗したときも、アプリをそのまま使い続けず、再起動して古い版でデータベースを開き直す（選び直せば、もう一度更新できる。入れ替えの失敗の理由はログにだけ出し、画面には出さない。閉じた後のデータベースを開き直す手段が無く、再起動した新しい画面から古い画面の状態を引き継げないため。同じ失敗が続くときは、次の確認でまた通知が出るので、再導入（4章）を案内する）。Windows では入れ替え（④）の中でアプリが終わる（プラグインの仕様）ので、③を先に明示的に行う。
- **ダウンロードの進み具合**は、通知を開いた表示の中に「ダウンロードしています 42%」の文で出す（大きさが分からないときは割合なし）。進んでいる間は「更新して再起動」を押せなくする。進み具合は割合が変わったときだけ画面に知らせる。
- **更新前のバックアップ**は、新しい版がデータベースを開くときの、マイグレーション前のバックアップ（`backups/`。アーキテクチャ設計書 3章）が受け持つ。更新専用のバックアップは作らない（マイグレーションが要らない更新では、データベースを変えないため。日次バックアップは別に動いている）。
- **署名に版を入れる：** 設定 `requireSignedVersion` を有効にし、署名が持つ版と `latest.json` の `version` が違う更新を断る。署名だけでは `latest.json` の版が署名された版かどうかが分からず、古い（署名は正しい）版へ誘導されるおそれがあるため。このため、署名は版を記録する Tauri の CLI で付ける（実機で確かめる。テスト計画書 4.9）。
- **Windows** は `installMode: passive`（進み具合だけが出て、操作は要らない）。
- **`latest.json` の形**は Tauri の静的な形（`version`・`notes`・`pub_date`・`platforms`）にする。`platforms` のキーは `darwin-aarch64`・`darwin-x86_64`・`linux-x86_64`・`linux-aarch64`・`windows-x86_64`・`windows-aarch64`、値は `url`（`https`。配布物の取得元）と `signature`（`.sig` の中身）。インストールスクリプトが読む `version`（4章）と同じファイルにする。

### 5.2 公開鍵と配布元の設定（オーナーの作業）

配布元は決まった（公開リポジトリは `misty-swan/yunagi`、公式サイトは Cloudflare Pages の `getyunagi.com`。1章・2章。2026-11-06）が、公開鍵はまだ無い（署名鍵の作成が未。YUN-100）。このため `src-tauri/tauri.conf.json` の `plugins.updater` は空のまま置いてあり、**空の間は更新の確認をしない**（エラーにも通知にもしない）。入れる値は次の 2 つで、どちらも空でなくなったときから確認が始まる。

- `pubkey`：更新署名鍵（6.1）の公開鍵（`tauri signer generate` が作る `.pub` の中身）。公開鍵は秘密ではないので、リポジトリに置く。
- `endpoints`：`["https://getyunagi.com/latest.json"]`。`https` 以外はプラグインが断る。**値はサイトを公開する前の今は入れず、空のまま置く**（2026-11-06。YUN-11 の判断）。`pubkey` が空の間は、`endpoints` に値があっても確認しない（`updater.rs` の `is_configured` が両方を要求する。試験が確かめる）ので、今入れても動きは変わらないが、`pubkey` を入れた（YUN-100）ときに、サイトの公開前でも確認が始まる恐れがある。まだ有効でない値を先に入れないため、`pubkey` と同じときに入れる。この値を入れた版を配る時点で、サイトと最初の `latest.json` が公開されていること（無いと、利用者のアプリが 24 時間ごとに失敗をログに残し続ける。5.1）。このため、サイトの公開（YUN-107）より前に配る版には入れず、最初のリリースでは、Release の配布物を確かめたあと、続けて `latest.json` を公開する（7章）。

手順（鍵と配布元が決まったときに、オーナーが行う。Linear の YUN-100 が追う。②の `.sig` と `latest.json` を作るのは配布用 CI の署名と公開（YUN-101。3.4）で、完了の条件に書いてある）：①`tauri signer generate` で鍵を作り、秘密鍵とパスフレーズを CI のシークレットとオフラインのバックアップに置く（6.1）。②配布用のビルドの設定（`src-tauri/release/common.json`。3.2）に `bundle.createUpdaterArtifacts` を有効にする（配布用 CI で署名ファイル `.sig` を作る。`tauri.conf.json` には入れない：入れると、鍵の無い手元のビルドが失敗する）。③`plugins.updater.pubkey` と `endpoints` に値を入れる。④配布用 CI が、署名と `latest.json` を公開する。窓の無い試験は、配る設定ファイルの `plugins.updater` がプラグインに読めること（`https` のみ・署名に版を入れる設定・Windows の入れ替え方）を確かめるので、③で形を誤ると `pnpm check` が止める。

プラグインの設定が誤っていて初期化できないときは、アプリは起動を続け、更新の確認だけが使えなくなる（ログに理由を出す）。

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
- [ ] バージョン番号を直下の `Cargo.toml` の `[workspace.package]` の `version` で上げた（版を書くのはここ 1 か所。`tauri.conf.json`・`package.json` は版を持たない。README.md）。タグは `v<この版>` にする（違えば配布用 CI が中止する。3 の 1）
- [ ] テスト計画書のリリース前テストを完了した
- [ ] 配布用 CI の検査ジョブ（`verify`）が成功した：許可外ライセンスと既知脆弱性が無く、許可リストの期限内で、`THIRD_PARTY_LICENSES.md` が生成された（3.1）。検査が「検査できなかった」で止まったときは、つなぎ直して再実行する。あわせて、許可リスト（`scripts/release/audit-allowlist.json`）の全件の理由がまだ成り立つことと、`cargo audit` の警告の一覧に新しいもの（特に `unsound`）が増えていないことを目で見た（3.1「警告の扱い」）
- [ ] 型チェックの道具（svelte-check）の新しい版が TypeScript 7 系に正式に対応したかを、アーキテクチャ設計書 3.2 の目安で確かめた（対応していれば移行を判断する）
- [ ] タグを作成して push した
- [ ] CI の全ジョブが成功し、配布物・`.sha256`・`.sig`・`SHA256SUMS`・`SHA256SUMS.sig` が揃っている（`.sig` 以降は YUN-101 が作る。3.4。YUN-97 の範囲では、全 6 組のビルドが成功し、配布物と `.sha256` が成果物に揃っているまで）
- [ ] 公開リポジトリの Release の配布物（`.sha256`・`.sig`・`SHA256SUMS`・`SHA256SUMS.sig` を含む）が、`<配布元>/v<ver>/<ファイル名>` から実際に取得できる（`latest.json` を更新する前に確かめる。先に更新すると、更新が 404 で失敗する）
- [ ] （最初のリリースの前に）`support@getyunagi.com`・`security@getyunagi.com` にテストのメールを送り、オーナーの受信箱に届くことを確かめた（Cloudflare の Email Routing。届かないと、アプリの画面・SECURITY.md・プライバシーポリシーが案内する窓口が使えず、脆弱性の報告も届かない）
- [ ] 公式サイトのデプロイ（Cloudflare Pages。2章「公式サイトの置き場所」）で `https://getyunagi.com/latest.json` を新しい版に更新し、`curl -fsS` で取った `version` が新しい版であることを確かめた
- [ ] （2 回目のリリースから）Pages の「Deployments」に、直前の版の `latest.json` を載せた本番のデプロイが残っていて、そのデプロイの URL（またはデプロイ ID）と公開日時を、リリースの記録に控えた（取り下げ時の戻し先。2章。最初のリリースには戻し先が無い）
- [ ] 配布する全 OS・アーキテクチャで、インストールスクリプトによる新規導入と、旧版からの自動更新を確認した（検証できなかったものは「試験提供」と明記した）
- [ ] Homebrew tap の更新がマージされ、Cask で導入したアプリが起動することを確認した
- [ ] 公式サイトのダウンロード案内と変更履歴を更新した

最初のリリースの前に、次も確かめる（初回だけ）：

- [ ] リポジトリを公開した（YUN-109）。公開後に macOS・Windows の CI と配布用のビルドが走ることを確かめた（テスト計画書 2章）
- [ ] 公式サイト（YUN-107）が `getyunagi.com` で見られ、HTTPS の証明書が有効になっている。`dig CAA getyunagi.com` で、証明書の発行を妨げる CAA 記録が無いことを確かめた（2章）
- [ ] Cloudflare のアカウントに 2 要素認証がかかっている。公開用の API トークンは Pages の編集だけの権限で、GitHub Actions の secrets に置いてある（2章）
- [ ] 取り下げの戻し方（2章）を、サイトの本番のデプロイを 2 つ以上作ったあと（最初のリリースの前に、試しの `latest.json` で 2 回デプロイしてよい）に 1 度試し、`latest.json` が前のデプロイの内容に戻り、公開用のワークフローを止める手順も確かめた
