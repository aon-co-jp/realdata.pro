# 開発方針(realdata.pro)

## やさしい要約 / Plain-language summary

### 日本語(AIと開発者への約束ごと)
1. **こわさない**: `cargo fmt --all` は使いません(ほかのリポジトリまで書きかえてしまうため)。かわりに `cargo fmt-own` / `lint-own` / `test-own` を使います。
2. **先に道具をそろえる**: ビルドの前に `scripts\fetch-deps.ps1`(Windows)か `scripts/fetch-deps.sh` で `.deps/` を作ります。`deps.lock` が「どの版を使うか」を決めています。
3. **秘密は見せない**: トークンやパスワードは `F:\` 直下のtxtから読み、画面にもコミットにも出しません。
4. **RESTは作らない**: 画面やcurlから呼ぶAPIはConnect(`POST /パッケージ.Service/Method`)だけにします。
5. **うまくいったか、本物で確かめる**: テストが通っただけで「できた」と言わず、本番(VPS)で動きを見てから報告します。
6. **VPSを再起動する前に**: 毎朝7時から約3時間半は自動収集が動いています。その間は再起動しません。
7. **ことば**: 人への返事は日本語だけで書きます。

**いまの状態(2026-11-19)**: 毎朝の自動収集は動いていて、直近は23か所すべてで集まりました(うち1か所は「該当なし」)。再起動しても「直近の収集」の表示がもどります。のこりの仕事は、検索元ごとの品質調整・日英中以外の言語・各社の無料枠の毎朝確認です。

### English (simple and friendly)
Rules for the AI and for developers:
1. **Do not break things**: Never run `cargo fmt --all`. It can rewrite other repositories too. Use `cargo fmt-own`, `lint-own` and `test-own` instead.
2. **Get the parts first**: Before you build, run `scripts\fetch-deps.ps1` (Windows) or `scripts/fetch-deps.sh` to make the `.deps/` folder. The file `deps.lock` says which version of each part to use.
3. **Keep secrets secret**: Read tokens and passwords from text files at the top of `F:\`. Never print them and never commit them.
4. **No new REST**: APIs that a page or `curl` calls use Connect only (`POST /package.Service/Method`).
5. **Check with the real thing**: Passing tests is not enough to say "done". Look at the real server (the VPS) first, and then report.
6. **Before restarting the VPS**: The daily crawl runs from 7:00 for about three and a half hours. Do not restart during that time.
7. **Language**: Reply to the person in Japanese only.

**Where we are (2026-11-19)**: The daily crawl is running. The latest run collected all 23 places (one of them was "nothing found"). After a restart, the "last crawl" box shows the latest run again. What is left: tuning the quality of each search source, languages other than Japanese, English and Chinese, and a daily check of each company's free plan.


作業ドライブは `F:\realdata.pro`。全リポジトリ共通ルールは
[`open-raid-z/CLAUDE.md`](https://github.com/aon-co-jp/open-raid-z) を正本として参照すること。

## このリポジトリの役割

SAS Viyaを参考にした、Rust製オープンソースのデータ分析プラットフォーム。大企業・中小企業・
起業する人の誰でも簡単に使えることを最優先にする。aon-co-jp エコシステム
(open-web-server・RPoem・open-cuda・aruaru-db・aruaru-llm・open-directx)の上に組み立てる。
公式ドメインは `realdata.pro`(ConoHa DNS: ns-a1/a2/a3.conoha.io、VPS 160.251.237.162 を予定)。

## 守ること

- **周りを壊さない**: 依存リポジトリは `deps.lock` でコミットを固定し、`scripts/fetch-deps` で `.deps/` へ取得する。
  `F:\` 直下の作業ツリーをパス依存しない。**`cargo fmt --all` は禁止**(依存先まで整形される)。
  `cargo fmt-own` / `cargo lint-own` / `cargo test-own` を使う。
- 依存を更新するときは、`deps.lock` の rev を書き換えて `fetch-deps` を実行し、テストが通ってからコミットする。
- 操作は GraphQL(`POST /graphql`)に集約し、REST エンドポイントを増やさない。
- `rrd-core` は外部クレートに依存させない。
- SAS社のコード・API・言語仕様を複製しない。READMEの商標注意書きを残す。
- 計画は [PORTING.md](PORTING.md)、普及計画は [docs/GO_TO_MARKET.md](docs/GO_TO_MARKET.md)。

## ビルドとテスト

```bash
bash scripts/fetch-deps.sh     # Windows: powershell -ExecutionPolicy Bypass -File scripts\fetch-deps.ps1
cargo fmt-own && cargo lint-own && cargo test-own
cargo run -p rrd-server        # http://127.0.0.1:4701/
```

## HANDOFF

### 🛑 再開用メッセージ(別アカウントで再開用、2026-11-17更新) / Resume message for another account

**日本語**

再開するリポジトリ: `realdata.pro`(手元`F:\realdata.pro`、VPSは`ssh conoha`の`/root/repository/realdata.pro`、サービス`realdata-pro.service`)。
関連: `aruaru-search`(`F:\aruaru-search`、検索元・セレクタ自己修復)、`aruaru-llm`(`F:\aruaru-llm`)、`RPoem`、`open-web-server`。メタ索引は`F:\runo`。
手元でcargoを使うときは`export PATH="$PATH:/c/Users/noruk/.cargo/bin"`(Git Bash)。VPSは`~/.cargo/bin/cargo`。秘密は`F:\`直下のtxt経由でscp、値は画面・コミットに出さない。

**確認済み(2026-11-08〜17)**
- 毎朝7:00 JSTの自動収集は既定(23か所・検索600件)で動いている。11/14=23か所成功、11/15=22か所成功・1か所失敗(山形県、検索語を含む結果が0件)、11/16・11/17=ともに23か所成功・失敗0(山形県も正常)。保存は`daily_jp_YYYYMMDD`としてGitHubへ。
- 早期打ち切り(`crawl.rs`の「quota」/「free search」判定)は、**連続`MAX_UNAVAILABLE_STREAK`回**のときだけ打ち切る設計で、9/29の「23か所中2か所」問題は解消済み。`explain.rs`の`all_quota_exceeded`は収集とは無関係(AI説明機能の文言)。
- 保管庫のシャード自動お引越しは、使い捨てリポジトリ(しきい値約1KB、別ポート・別データ)で実地確認済み: `-archive-2`を自動作成して切替→旧シャードsealed→`-3`へ連鎖、`catalog.json`の索引も正しい。
- aruaru-searchの検索元: excite追加(日本語4つ)、セレクタ自己修復+3回連続の断念で自動無効化+探りによる自動再有効化(Baiduは自動復帰を確認)。

**未了・次にやること**
1. 本番の実シャード(`realdata-archive`)が約1GBに近づいたときの実際の引っ越し(ログ`保管庫 … が上限に近づきました`)を見守る。2026-11-17時点で約4.4MBで、まだ遠い(対応は当面不要)。
2. 使い捨てのGitHubリポジトリ4件(`rrd-rotation-test-archive`・`-2`・`-3`・`rrd-rotation-test-catalog`)は**2026-11-17に削除済み**(対応不要)。
3. 「4層4重」の記録は**2026-11-17に完了**(下の「open-web-server ⇔ RPoem の『4層4重』…」節)。残るのは、realdata.pro の日次保存に④独立監査ログ(保存名・サイズ・SHA-256・時刻を追記専用ファイルへ)等を足すかの**設計判断**(未実装)。**ユーザーに「`rrd-server`へ実装して本番反映してよいか」「①〜③(PostgreSQL・Ledger経由のaruaru-db・マルチリージョン)へどこまで広げるか」を確認してから着手する**(返事は未受領)。
4. 山形県の失敗は11/15の1回のみで、11/16以降は正常。繰り返すなら検索語・除外条件を見直す。11/16・11/17の「地図データを先に取得して保存」が0件(以前は28件)だった原因は未確認(取得済みで不要だっただけの可能性が高い)。
5. 毎朝の収集で、場所が23か所すべてに進むかを引き続き確認(失敗は`journalctl -u realdata-pro`)。

**落とし穴**: 7時ゲート(`main.rs`の`hour >= 7`)を外すテスト用の変更はコミットしない(本番のみ一時的に使い、戻し済み)。`cargo fmt --all`は禁止(パス依存の兄弟リポジトリまで整形される)、`cargo fmt -p <crate>`を使う。PowerShell 5.1で日本語ファイルを書かない。

**English**

Repositories to resume: `realdata.pro` (local `F:\realdata.pro`; VPS `/root/repository/realdata.pro` via `ssh conoha`; service `realdata-pro.service`). Related: `aruaru-search` (`F:\aruaru-search`: engines and selector self-repair), `aruaru-llm` (`F:\aruaru-llm`), `RPoem`, `open-web-server`. The meta index is `F:\runo`. Locally, use `export PATH="$PATH:/c/Users/noruk/.cargo/bin"` (Git Bash); on the VPS use `~/.cargo/bin/cargo`. Secrets go through txt files at the root of `F:\` and `scp`; never print values or commit them.

**Verified (2026-11-08 to 17)**
- The 7:00 JST daily crawl runs with the right defaults (23 places, 600 searches). 11/14: 23 places succeeded; 11/15: 22 succeeded, 1 failed (Yamagata: zero results containing the query terms); 11/16 and 11/17: 23 succeeded, 0 failed (Yamagata fine). Saved to GitHub as `daily_jp_YYYYMMDD`.
- The early break (`quota` / `free search` in `crawl.rs`) only fires after `MAX_UNAVAILABLE_STREAK` consecutive failures, so the 9/29 "2 of 23 places" problem is fixed. `all_quota_exceeded` in `explain.rs` is unrelated to the crawl (it is a message of the AI explain feature).
- Automatic vault shard rotation was verified live with throwaway repos (threshold ~1 KB, separate port and data): `-archive-2` was created and writes switched, the old shard was sealed, it chained to `-3`, and the `catalog.json` index was correct.
- aruaru-search: excite added (4 Japanese engines); selector self-repair, auto-disable after 3 consecutive give-ups, and auto re-enable by probing (Baidu recovered by itself).

**Open / next**
1. Watch the real shard (`realdata-archive`) approach ~1 GB and rotate for real (log line `保管庫 … が上限に近づきました`). At 2026-11-17 it is about 4.4 MB, far from the limit (no action needed for now).
2. Four throwaway GitHub repos (`rrd-rotation-test-archive`, `-2`, `-3`, `rrd-rotation-test-catalog`) were **deleted on 2026-11-17** (nothing left to do).
3. Documenting the "4-layer, 4-fold" setup was **done on 2026-11-17** (section "open-web-server ⇔ RPoem …" below). What remains is the **design decision** whether to add a fourth-leg independent audit log etc. to realdata.pro's daily saves (not implemented). **Ask the user first**: may the audit log be implemented in `rrd-server` and deployed to production, and how far to extend (1)–(3) (PostgreSQL, aruaru-db via the Ledger, multi-region)? (No answer received yet.)
4. Yamagata failed only once (11/15) and was fine from 11/16. If it repeats, revisit the query terms / exclusion filter. Why the pre-fetched map data was 0 items on 11/16 and 11/17 (it was 28 before) is unconfirmed (probably already cached).
5. Keep checking that every morning's crawl reaches all 23 places (failures: `journalctl -u realdata-pro`).

**Pitfalls**: never commit the test-only change that bypasses the 7:00 gate (`hour >= 7` in `main.rs`; it was used on the VPS only and reverted). Do not run `cargo fmt --all` (it reformats path-dependent sibling repos); use `cargo fmt -p <crate>`. Do not write Japanese files from PowerShell 5.1.

---

(以下は2026-09-30時点の旧い再開用メッセージ。上の2026-11-17版が最新 / The text below is the older 2026-09-30 message; the 2026-11-17 one above is current.)

### 🛑 再開用メッセージ(次回このセッションを続ける人へ、2026-09-30更新)

**新しい発見(2026-09-30、要対応)**: 9/29 の実際の朝7時収集(既定600件・約22か所の設定で
実行された、7時ゲート自体は正しく動いた回)を GitHub の `realdata-archive` で確認したところ、
`daily_jp_20260929.csv` は **23か所中わずか2か所(熊本県・大分県、48件)で打ち切られていた**。
`crawl.rs:275`「検索エラーに `quota`/`free search` を含んだら即座にその日の収集を打ち切り、
残りは翌日に持ち越す」設計のため。同時刻の `aruaru-search` の `/v1/health` では Bing・Brave が
休止中(拒否直後の17分待機)、**Baidu は「結果を1件も読み取れません(ページの作りが変わった
可能性)」でセレクタ破損中**だった。項目3(日本語検索元が心許ない)は理論上の懸念ではなく、
**実際に収集量を数か所ぶんまで削っている**ことが実データで裏付けられた。次回はこの3点を順に:
(1) 今回の発見をドキュメント化・commit・push・VPS反映(このメッセージ自体がそれ)、
(2) Baidu のセレクタ修復(`maintain.rs` の自己修復が効いていない理由の確認含む)、
(3) `all_quota_exceeded`(`explain.rs:122`)が何を指すか(共有キーの1日100回制限か、
`aruaru-search` 自体のクライアント別レート制限か)をコードで特定し、本当に「打ち切って正解」
なのか、それとも早すぎる打ち切りなのかを見極める。

**残る未確認事項**: 下の「未確認・次回やること」の**項目2**(シャードの容量がしきい値1GBに
近づいての実地の自動引っ越し確認、容量がまだ遠いため様子見)。項目1(7:00 JST自動収集の既定
確認)は2026-09-29に確認済みだが、上記のとおり**実際の収集量は別の問題(検索元の拒否)で
細っている**ことが判明したため、項目3は「対応済み」から「調査継続」に戻す。

- **直近完了**: GitHub 保管庫(非公開リポジトリ)の**複数リポジトリ自動引っ越し**機能を実装し、
  本番(VPS)で有効化・実機確認まで完了した(下の「保管庫の複数リポジトリ自動引っ越し」節)。
  - 索引リポジトリ `aon-co-jp/realdata-catalog`(非公開)を新規作成済み。
  - VPS の `realdata-pro.service` に `RRD_CATALOG_REPO`・`RRD_GITHUB_TOKEN` を設定・再起動済み。
  - GraphQL 経由で保存→索引記録・削除→索引消去を実機で確認済み(テストデータは削除済み)。
- **同じセッションで見つけた別のバグ**: VPS の `realdata-pro.service` が、設定ファイルを直した後も
  **再起動していなかった**ため、試験用の環境変数(`RRD_CRAWL_FORCE=1`・`RRD_CRAWL_SEARCHES_PER_DAY=81`)
  を3日間(9/26〜9/28)引きずっていた。9/28 に発見・再起動して解消済み(下の「バグ調査」節)。
- **未確認・次回やること**:
  1. **確認済み(2026-09-29)**: 7時ゲート(`main.rs`の`hour >= 7`)を一時的にのみ環境変数で
     回避するテスト専用ビルドをVPS上だけで作って実行し、「毎朝の自動収集を開始します(場所 23か所、
     検索の組 600件、無料検索 使える)」と正しい既定(旧テスト値の3か所・81件ではない)で動くことを
     確認した。確認後は本番コード(7時ゲートあり)に戻して再ビルド・再起動済み(GitHubへの
     push・コミットはしていない、一時的なローカル検証のみ)。今日の実際の7:00 JST収集は
     通常どおり本番コードで別途実行される。
  2. 現在のシャード(`realdata-archive`)の容量がしきい値(既定1GB、2026-10-03変更)に近づいて、実際に自動引っ越し
     (新しいリポジトリの自動作成)が起きる場面は、まだ実地で確認できていない(容量的にまだ遠い)。
     容量が近づいてきたら、VPS のログ(`realdata.pro: 保管庫 … が上限に近づきました`)を確認する。
  3. 日本語検索元が Bing・Brave・Yahoo! JAPAN の3つのみで心許ない点は、2026-09-28〜29に
     `aruaru-search` へエキサイト ウェブ検索(id=`excite`)を追加して4つに増やした(実機確認済み)。
     goo検索・BIGLOBE・Infoseek・@nifty検索・Startpage・Yandex・Sogou・Ecosia・Qwant も調査したが、
     いずれもサービス終了・JS描画・robots.txt禁止・ボット検知のいずれかで見送り(詳細は
     `aruaru-search/CLAUDE.md`「日本語検索元の追加調査」節)。根本的な解消ではないため、引き続き課題。
     **2026-09-30追記**: 実データで裏付け完了(上の「新しい発見」節)。9/29の実収集は
     23か所中2か所で打ち切り。Baidu セレクタ破損・quota打ち切りロジックの調査が次の対応。
- 詳細な経緯は、下の「保管庫の複数リポジトリ自動引っ越し」「バグ調査」「試験収集の結果」の各節を参照。

- **2026-09-24 P0**: `rrd-core`(列指向エンジン)と CLI。
- **2026-09-24 P1**: ユーザー指示「Rust + open-web-server・open-directx・aruaru-llm・aruaru-db・open-cuda・
  RPoem を中心に作る」を受けて、次を実装した。
  - `rrd-compute`: open-cuda の sgemm で重回帰
  - `rrd-server`: RPoem 上の GraphQL と Web UI
  - ブラウザで一連の操作(読込→クレンジング→集計・棒グラフ→重回帰→入力エラー表示)が動くことを確認した。
    重回帰の係数は、厳密な有理数計算と一致した。
  - 同じセッションで `cargo fmt --all` の事故が起きた。これを受けて、依存を固定して取得する設計(`deps.lock` / `.deps/`)に切り替えた。
  - 次は P2(aruaru-db)→ P3(aruaru-llm)→ P4(open-directx)→ P5(realdata.pro 公開)。
- **2026-09-24 続き**:
  - ユーザー指示により、リポジトリ名を `rs-real-data` から `realdata.pro` に変更した(GitHub・ローカルフォルダとも)。
  - 取り込み元に、検索ワード(Google/YouTube/GitHub)と調査対象 URL を追加した。
  - aruaru-llm での多言語説明(日本語・英語と、約130言語から選んだ1言語)を追加した。
  - aruaru-llm 側には `POST /v1/search/raw` を新設した。
  - キーは aruaru-llm の設定を正本として共有する(PORTING.md「API キーの共有」)。
  - GitHub 検索・URL 取り込み・SSRF 拒否・3言語の説明は、ブラウザで確認済み。
  - Google 検索は、開発機のキーでは 403 だった。VPS で要確認。
- **2026-09-24 続き2**:
  - P4(open-directx の GPU 描画+AVX-512/AVX2 の CPU ラスタライザ)を実装した。
    開発機では GPU と CPU の出力が完全一致した。AVX-512 の経路は VPS(Xeon Icelake)で、スカラーと画素単位で一致した。
  - 世界リサーチ(`research.rs`)を実装し、実データで7回の開発・テスト・デバッグを行った(PORTING.md に記録)。
  - aruaru-llm に `/v1/search/raw` の `gl`/`hl` 対応を追加し、VPS に反映済み。
  - CI は `F:\git.txt` のトークンで push した(このトークンは workflow 権限付き。値は表示・保存しない)。
  - realdata.pro のサーバー自体は、まだ VPS にデプロイしていない(DNS の設定待ち)。
- **2026-09-24 続き3**:
  - VPS にデプロイ済み: https://realdata.pro/(`realdata-pro.service`、`127.0.0.1:4701`、open-web-server の背後、Let's Encrypt)。
    DNS は a.conoha-dns.com / b.conoha-dns.org で反映済み。
  - 全ドメインの certbot 更新が失敗していた障害を修正した(open-web-server の `CLAUDE.md` に詳細)。
  - 起業・企業向けサービス(サンプル)、資金運用の公的データ、外貨定期預金金利の毎朝7時の自動収集、求人リンクを追加した。
  - P2(aruaru-db による保存・版管理)を実装し、VPS では専用の aruaru-db(`realdata-aruaru-db.service`)で稼働している。
  - aruaru-db の INSERT 解析の実バグを発見した(値内の `,` `)` 引用符)。別作業に切り出し、こちらは Base64 で回避している。
  - 未実施: YouTube のキー、open-directx のデスクトップビューア、GPU バックエンド(VPS には GPU が無い)。
- realdata.pro は、作成時点で DNS の委任が完了していない(→ 上記のとおり解消済み)(レジストリに NS が未登録、ConoHa DNS にゾーンが未作成)。
- `.github/workflows/ci.yml` は、gh トークンに `workflow` 権限が付くまで push できない。

## 保存先・検索の上限・地図データ(2026-09-25 ユーザー指示)

- **保存先は GitHub の非公開リポジトリ**(`RRD_ARCHIVE_REPO`、現在は `aon-co-jp/realdata-archive`)。VPS の DB(aruaru-db)・ディスクには保存しない。3日などの保持期限は無く、保存できたものはすべて GitHub にある(保存に失敗したものだけ手元に置き、翌日送り直す)。データセットは `datasets/`、毎朝の自動収集は `daily/<年>/`、地図データのキャッシュは `osm/`。`git log` が版の履歴(`store.rs` / `archive.rs`)。
- **1日の検索の上限は3,000件**(毎朝の自動収集と日中の世界リサーチの合計、`research::MAX_SEARCHES_PER_DAY`)。毎朝の自動収集の分は2,500件(日中の世界リサーチのために500件を残す)。根拠: 1検索≒3件≒2.4KB(圧縮後≒0.7KB)で、3,000件/日なら1年≒0.8GB(GitHub の推奨は1リポジトリ1GB未満)。実測して見直す。
- **毎朝の自動収集は無料の自前メタ検索(aruaru-search)だけを使う**(`free_only`)。共有キーの検索(1日100回)へは移らない。使えないときは1日20件。
- **地図データ(OpenStreetMap)は夜(日本時間1〜6時)に先取得**して `osm/` に保存し(`osm::refresh`、1日30件、90日で一巡)、世界リサーチでは保存済みを使う。数の多い分類をその場で問い合わせると時間切れになるため。
- `RRD_CRAWL_FORCE=1` を付けて起動すると、今日のぶんがあっても、起動後に1回だけ毎朝の自動収集を実行する(動作確認用)。動作確認のあとは外すこと。

## open-web-server ⇔ RPoem の「4層4重」通信・DB連携(2026-11-17 調査・確認、日英)/ "4-layer, 4-fold" communication and DB integration

### 日本語

**役割**: `open-web-server` = Apache(+Nginx)相当の前面Webサーバー、`RPoem` = Tomcat相当のアプリケーションサーバー。一般的な「Apache→Tomcat→DB」の3層構成(Apacheが静的配信と負荷分散、Tomcatがアプリ、DBが永続化。[Web Application Clusters](https://mockflow.com/examples/web-application-cluster-architecture-diagram-using-ideaboard/)、[Atlassian: Apache と Tomcat の接続](https://ja.confluence.atlassian.com/x/p4nOD))を土台にしつつ、本プロジェクトの「4層4重」は**独自の目標アーキテクチャ**で、定義の正本は[`open-web-server/CLAUDE.md`](https://github.com/aon-co-jp/open-web-server)「拡張要件(2026-07-11)」節(出典つき、日英Web検索で裏取り済み)。ここへ要約する。

**4層(設計の4本柱)**
1. **VersionLess API × バージョン管理のハイブリッド**: APIはバージョンレス、データは `aruaru-db`(Git-on-SQL)でコミット単位に版管理。
2. **Git管理 + ディスク冗長化**: 全変更をコミットで記録し、`open-raid-z`(ZFS類似の冗長化)を永続化層に。Gitの履歴とは独立に、ファイルシステムのスナップショットが別系統の復元手段になる。
3. **通信層の四重化**: ①TCP-IP(権威パス)②UDP-IP(即時通知)③QUIC(MPQUICは範囲外)④MPTCP/SCTP(カーネル実装が使えない環境ではユーザー空間の`aggligator`で代替。実金融機関の物理経路冗長化はSD-WANが主流で、これは**次善策**と明記済み)。`open-web-server-wire`。
4. **DB書き込みの四重化**: ①PostgreSQL ②aruaru-db ③マルチリージョン同期レプリケーション ④独立した監査用トランザクションログ(①〜③の複製ではなく、突き合わせ用の独立した検証手段)。`open-web-server-ledger`。

別軸として、`open-web-server-wire` の**4層防御通信**(TLS 1.3 → 相互認証 → AEAD暗号化 → seq/timestampリプレイ対策)がある。これは伝送路の冗長化(上の3)とは別のセキュリティ層。

**通信とDBの接続点(経路)**: クライアント → `open-web-server`(wire) → `Ledger`(冪等WAL+3ホップコミット+独立監査ログ) → RPoemゲートウェイ(`open-runo-gateway`、VPSでは127.0.0.1:8140) の Connect `Admin/DbMutate`(`x-api-key`必須。環境変数 `OPEN_RUNO_API_KEY`) → `aruaru-db`。`db_commit_id`をクライアントへ返し、同じキーの再送は同一の受領票(冪等)。

**確認できた現状(2026-11-17)**
- 実装・検証済み: Ledgerの冪等WAL・監査ログ・マルチリージョン複製(実SQLite2リージョン)・PostgreSQL WAL(WSL2のライブPostgreSQLで検証)、wireのTCP/UDP/QUIC(単一経路)と、ユーザー空間の多経路集約。VPSでは、LedgerとRPoemゲートウェイの配線(11-14)と、VPS上での同形のDbMutate実送信(`db_commit_id`返却・同キー再送で同一受領票)を確認済み。
- 未了: 実際のLedger書き込みの本番通し、(1)の読み出し側(commit_id指定の過去状態照会API)、MPQUIC、カーネルMPTCP/SCTP、RPoemゲートウェイのHTTP/2(前面はHTTP/1.1のみ)。
- **`realdata.pro` との関係(正直な現状)**: `rrd-server`(127.0.0.1:4701)は**この4層4重の書き込み経路(Ledger→DbMutate)を使っていない**。収集データはGitHubの非公開保管庫(複数シャード+`catalog.json`)へ保存し、`realdata-aruaru-db`(専用プロセス、版管理用)も別に持つ。つまり realdata.pro は上記のうち「前面(open-web-server)の背後で動く1アプリ」にすぎず、4重の書き込みを受ける対象には**まだなっていない**。

**今後(未実装・要判断)**: realdata.pro の日次保存に、④独立監査ログ(保存名・サイズ・SHA-256・時刻を追記専用ファイルへ)を足すのが、最小で実害のない第一歩。①〜③(PostgreSQL・Ledger経由のaruaru-db・マルチリージョン)をどこまで適用するかは設計判断が要る。

出典: [open-web-server/CLAUDE.md](https://github.com/aon-co-jp/open-web-server)、[CMT-SCTP and MPTCP](https://www.mdpi.com/2079-9292/11/15/2384)、[Multipath QUIC (CoNEXT 2017)](https://dl.acm.org/doi/10.1145/3143361.3143370)、[Cisco: SD-WAN for Financial Services](https://www.cisco.com/c/dam/m/en_us/solutions/industries/financial-services/new-financial-services-landscape-of-networking/SD-WAN-for-Financial-Services.pdf)、[Azure: Designing for redundancy](https://learn.microsoft.com/en-us/azure/well-architected/reliability/redundancy)、[PostgreSQL on ZFS](https://sumguy.com/postgresql-on-zfs-tuning-snapshots/)。2026-11-17の再検索では、「独立した監査ログを主DBと別に持つ」設計そのものを裏づける独立した資料は見つからなかった(ベンダー資料のみ)。この点は設計上の判断であり、外部の標準ではない。

### English

**Roles**: `open-web-server` = the front web server (Apache/Nginx-like), `RPoem` = the application server (Tomcat-like). The common "Apache → Tomcat → DB" 3-tier layout (Apache serves static files and balances load, Tomcat runs the app, the DB persists; see [Web Application Clusters](https://mockflow.com/examples/web-application-cluster-architecture-diagram-using-ideaboard/) and [Atlassian: connecting Apache to Tomcat](https://ja.confluence.atlassian.com/x/p4nOD)) is the base. This project's "4-layer, 4-fold" is **its own target architecture**; the source of truth is the "Extended requirements (2026-07-11)" section of [`open-web-server/CLAUDE.md`](https://github.com/aon-co-jp/open-web-server) (cited, verified by Japanese and English web searches). Summary:

**The four layers**
1. **VersionLess API × version control hybrid**: the API is versionless; data is versioned per commit in `aruaru-db` (Git-on-SQL).
2. **Git + disk redundancy**: every change is a commit; `open-raid-z` (ZFS-like redundancy) is the persistence layer, and filesystem snapshots give a second, independent restore path besides Git history.
3. **Four-fold transport**: (1) TCP-IP (authoritative path), (2) UDP-IP (instant notification), (3) QUIC (MPQUIC out of scope), (4) MPTCP/SCTP (where kernel support is unavailable, the user-space `aggligator` is used; real financial institutions mostly use SD-WAN for path redundancy, so this is documented as a **second-best substitute**). Crate: `open-web-server-wire`.
4. **Four-fold DB writes**: (1) PostgreSQL, (2) aruaru-db, (3) multi-region synchronous replication, (4) an independent audit transaction log (not a copy of 1–3 but an independent means of cross-checking). Crate: `open-web-server-ledger`.

A separate axis: the **4-layer defensive channel** in `open-web-server-wire` (TLS 1.3 → mutual auth → AEAD encryption → seq/timestamp replay protection). It is a security layer, distinct from the transport redundancy in item 3.

**Where communication meets the DB**: client → `open-web-server` (wire) → `Ledger` (idempotent WAL + 3-hop commit + independent audit log) → the RPoem gateway (`open-runo-gateway`, 127.0.0.1:8140 on the VPS) via Connect `Admin/DbMutate` (`x-api-key` required; env `OPEN_RUNO_API_KEY`) → `aruaru-db`. The `db_commit_id` is returned to the client, and a resend with the same key returns the same receipt (idempotent).

**Verified status (2026-11-17)**
- Implemented and tested: Ledger idempotent WAL, audit log, multi-region replication (two real SQLite regions), PostgreSQL WAL (against a live PostgreSQL in WSL2); wire TCP/UDP/QUIC (single path) and user-space multipath aggregation. On the VPS: the Ledger→RPoem gateway wiring (11-14) and a real DbMutate send of the same shape as the Ledger's (returns `db_commit_id`; a resend returns the same receipt).
- Open: a real production Ledger write end to end, the read side of layer 1 (query past state by `commit_id`), MPQUIC, kernel MPTCP/SCTP, HTTP/2 on the RPoem gateway (the front is HTTP/1.1 only).
- **Relation to `realdata.pro` (honest status)**: `rrd-server` (127.0.0.1:4701) **does not use this 4-layer, 4-fold write path (Ledger → DbMutate)**. It stores collected data in private GitHub vault repositories (several shards plus `catalog.json`) and has its own `realdata-aruaru-db` process (for versioning). So realdata.pro is just one application running behind the front server; it is **not yet** a target of the four-fold writes.

**Next (not implemented, needs a decision)**: the smallest harmless first step is a fourth-leg independent audit log for realdata.pro's daily saves (name, size, SHA-256 and time appended to an append-only file). How far to apply (1)–(3) (PostgreSQL, aruaru-db via the Ledger, multi-region) is a design decision.

A fresh search on 2026-11-17 found no independent source supporting the specific idea of keeping an audit log independent of the primary DB (vendor material only). That is a design choice of this project, not an external standard.


## 保管庫の複数リポジトリ自動引っ越し(2026-09-28、2026-10-14追記)

**お引越しの確認手順(2026-10-14)**: 本番の現シャードは約0.9MBで、実際のしきい値到達はまだ先。実地の通し確認は、
本番とは別の使い捨て非公開リポジトリと極小しきい値(`RRD_ARCHIVE_THRESHOLD_GB=0.000001`)の検証用インスタンスで行う
(手順は [PORTING.md](PORTING.md)「お引越しの手動確認手順」)。本番のシャードには書かない。

**English summary**: Archive rotation ("hikkoshi") creates a new private shard when the current one nears the threshold
(default 1GB, or predicted within 14 days) and switches writes; old shards stay read-only. Enabled by `RRD_CATALOG_REPO`.
Production shard is ~0.9MB, so rotation has not yet occurred; verify end-to-end with a throwaway shard and a tiny
threshold, never the production shard (see PORTING.md).

GitHubの1リポジトリの容量上限に備え、`RRD_CATALOG_REPO`(索引リポジトリ)を設定すると、現在のシャード
(`RRD_ARCHIVE_REPO`)の容量が既定1GB(2026-10-03に8GBから変更。GitHub推奨は1GB未満・ソフト上限5GB。`RRD_ARCHIVE_THRESHOLD_GB`で変更可)に達する見込みになったとき(増加ペースから14日以内と予測した時点で次のシャードを先行作成、切り替えはしきい値到達時)、GitHub API
(`RRD_GITHUB_TOKEN`または`GITHUB_TOKEN`)で新しい非公開リポジトリを自動作成し、書き込み先を自動的に
切り替える(`vault.rs`)。未設定なら、これまでどおり単一リポジトリのみで動く。詳細は
[PORTING.md](PORTING.md)「保管庫の複数リポジトリ自動引っ越し」を参照。開発機でのビルド・lint・
テスト(45件)はすべて通過済み。

**本番で有効化済み(2026-09-28)**: 索引リポジトリ `aon-co-jp/realdata-catalog`(非公開)を新規作成し、
VPS の `realdata-pro.service` に `RRD_CATALOG_REPO` を設定・`RRD_GITHUB_TOKEN`(既存の gh CLI トークン、
`repo` scope)を `.env.realdata` に追加して再起動、有効化を確認した(起動ログに「保管庫の自動引っ越しが
有効です」)。GraphQL 経由で実際に保存(`saveDataset`)→ `catalog.json` にシャードとパスの索引が記録
されたこと、削除(`dropDataset`)→ 索引が消えたことを実機で確認済み(テスト用データセットは削除済み)。
現在のシャードは `realdata-archive` 1つのみ(容量はしきい値に遠く及ばず、まだ引っ越しは起きていない。
実際に容量が近づいての自動作成・引っ越し自体は今後の運用の中で確認する)。

## 試験収集の結果(2026-09-26)

- **確認できたこと**: 試験収集の1か所目(岩手県)は、27件の検索で70件を収集できた(以前は同じ場所が「1件も集められない」で失敗していた)。
  修正版では、拒否されたり検索語の一部しか反映されなかったりする Bing と Brave は1時間休止し、
  Yahoo! JAPAN・Daum・Baidu・Naver は使えている。自動収集は共有検索の枠を使わなくなった。
- **まだ確認できていないこと**: 3か所の試験収集は、残り2か所が実行中。1か所に約20分かかり、
  日本全国48か所を回るには時間がかかるため既定を1日600件(約22か所)にしている。全国を一巡するには約2日かかる計算。
  全体の成功率はまだ見ていない。
- **設定の後始末**: 試験用の設定(強制実行・件数81・デバッグログ)は設定ファイルから外した。
  現在動いている試験収集はそのまま完了させる。次にVPSを再起動しても、強制実行は行われない。
- **今後の課題**: 日本語の検索元が Bing・Brave・Yahoo! JAPAN の3つしかなく、休止が重なると細くなる。
  他の検索元も試したが、いずれも拒否されるか結果の形が使えなかった。対象の検索元を増やす取り組みは引き続き必要。
  翌朝、自動収集が1日600件の設定で成功するかを確認し、結果を報告する。

## バグ調査: 「27件の検索で70件収集」は正常、ただし別の実バグを発見(2026-09-28)

- ユーザーからの指摘: 「1日600件中、70件しか集められなかったのか」。確認の結果、**表現の誤解であり、
  この件自体はバグではない**。600件は「1日に使う検索の組の数」(27種類の知りたい情報 × 場所)の上限で、
  「70件」は**そのうち1か所ぶん(27件の検索)から集められた記事数**(理論上の最大 27×3=81件に対して86%)。
  600件全体に対する到達率ではない。ドキュメントの書き方が紛らわしかったため、上の節に注記を追加した。
- **ただし、調査の過程で別の実バグを発見した**: VPS の `realdata-pro.service` が、2026-09-25 16:15 JST の
  起動時点の環境変数(試験用の `RRD_CRAWL_FORCE=1` / `RRD_CRAWL_SEARCHES_PER_DAY=81`)を**プロセスに
  保持したまま**、3日間(9/26・9/27・9/28)ずっと動き続けていた。設定ファイル(`.env.realdata`・
  systemd drop-in)からは試験用の値を外していたが、**サービスを再起動していなかった**ため、
  「今動いているプロセス」には反映されず、毎朝の自動収集が既定の600件(約22か所)ではなく、
  試験用の81件(3か所のみ)で動き続けていた。
  - `/proc/<pid>/environ` で確認し、`systemctl restart realdata-pro.service` で解消(2026-09-28)。
    再起動後の環境変数には `RRD_CRAWL_FORCE` / `RRD_CRAWL_SEARCHES_PER_DAY` が無いことを確認済み。
  - 翌日(2026-09-29)7:00 JST の自動収集から、正しい既定(600件・約22か所)で動くはず。結果は要確認。
  - 教訓: 設定ファイルを直しただけでは、動いているプロセスの環境変数は変わらない。**環境変数がらみの
    設定変更は、必ずサービスを再起動するところまで確認する**(または起動時に読み込む値をログに出力し、
    起動後に照合する)。

### Bug investigation: "70 of 27 searches" is normal, but a real bug was found elsewhere (2026-09-28, English)

- User question: "did you only manage to collect 70 out of a 600/day target?" On investigation, **this
  specific point was a misunderstanding of the wording, not a bug**: 600 is the daily budget of *search
  units* (27 topics × location), and "70" was the number of articles gathered from *one* location's 27
  searches (86% of the theoretical max of 27×3=81) — not a 70/600 shortfall. The docs wording was clarified
  above.
- **However, the investigation surfaced an actual bug**: the VPS's `realdata-pro.service` had been running
  continuously since 2026-09-25 16:15 JST with the test-only environment variables it was started with
  (`RRD_CRAWL_FORCE=1`, `RRD_CRAWL_SEARCHES_PER_DAY=81`) still held in the live process, even though those
  values had been removed from the config files (`.env.realdata`, the systemd drop-in) — **the service was
  never restarted**, so for 3 days (9/26, 9/27, 9/28) daily automatic collection kept running at the
  test setting of 81 searches (3 locations) instead of the intended default of 600 (~22 locations).
  - Confirmed via `/proc/<pid>/environ`; resolved with `systemctl restart realdata-pro.service`
    (2026-09-28). Verified the restarted process's environment no longer has those variables.
  - Tomorrow's (2026-09-29) 7:00 JST run should be the first to use the correct default (600 searches,
    ~22 locations). Outcome to be confirmed.
  - Lesson: editing config files alone does not change a running process's environment. **Any
    environment-variable config change must be followed through to a service restart** (or the startup
    values should be logged and cross-checked after the fact).

### Test collection results (2026-09-26, English)

- **Confirmed**: The first test-collection location (Iwate) gathered 70 records from 27 searches
  (previously this same location failed with "0 records collected"). In the fixed version, Bing and Brave
  — which were either rejected or only honored part of the query — are suspended for 1 hour, while
  Yahoo! JAPAN, Daum, Baidu, and Naver are working. Automatic collection no longer consumes the shared
  search quota.
- **Not yet confirmed**: Of the 3 test locations, 2 are still running. One location took about 20 minutes;
  covering all 48 locations in Japan takes a long time, so the daily default is set to 600 searches
  (~22 locations). A full pass over the whole country is estimated at ~2 days. Overall success rate has
  not yet been measured.
- **Cleanup**: Test-only settings (forced run, count 81, debug logging) have been removed from the config
  file. The test collection currently running will be allowed to finish as-is; the next VPS restart will
  not trigger a forced run.
- **Remaining work**: Only 3 Japanese-language search sources (Bing, Brave, Yahoo! JAPAN) exist, so
  overlapping suspensions thin out coverage. Other sources tried were either rejected or returned unusable
  result formats. Adding more search sources remains necessary. Tomorrow morning, we will confirm whether
  automatic collection succeeds under the 600/day setting and report the outcome.

## JSON処理の使用方針: `parse_secure_sonic`(速い型+安全)と`sonic-rs`は使える所では全て使う(2026-10-14、ユーザー指示)
正本は[`RS-JSON`](https://github.com/aon-co-jp/RS-JSON)(`README.md`・`PORTING.md`)。

**方針**: JSONを読む所は、使える所では**全て** `rust-json`(RS-JSON)の `parse_secure_sonic`(`fast`機能、結果は`sonic_rs::Value`)を使う。
`sonic-rs` を直接使える所(型付き構造体への読み込みなど)も同様。使えない所は `parse_secure`(結果は`serde_json::Value`)を使う。
外部から来る信頼できない入力(HTTP本文・ネットワーク越しのファイル・アップロード)は、**必ずサイズ・深さ・重複キーの上限つき**で読む。

**実測(7 MB・4万件)**: `serde_json` 200 ms / `parse_secure` 217 ms / **`parse_secure_sonic` 83 ms(約2.4倍速い、防御込み)** / `sonic-rs`そのまま 33 ms(約6倍、防御なし)。
`serde_json::Value`に読む限りSIMDでもほぼ速くならない(188 ms)。**速さの源は、結果を連続した領域に詰め込む値の型**。

**使える所 / 使えない所**
- ✅ 読み取り中心で、JSONを大きく・頻繁に読む所(キャッシュJSON、設定、APIの応答、ログ、インデックス)。
- ✅ 外部からの入力の入口(速さより防御が目的。`parse_secure`でも可)。
- ⚠️ `serde_json::Value`を関数間で受け渡している既存コードは、型を`sonic_rs::Value`に直す書き換えが要る(`JsonValueTrait`/`JsonContainerTrait`のメソッド)。書き換えずに`serde_json::Value`へ読むと速くならない。
- ❌ WASM(`rust_json::light`、依存ゼロ)、PHP/JavaScript側のJSON処理(Rustの部品は使えない)。
- 注意: 依存先の`sonic-rs`は内部で`unsafe`を使う(自前コードに`unsafe`は無い)。小さなJSON(数KB)では体感差が出ない。速度目的の書き換えは、実測で効く所から。

**導入**: `Cargo.toml`に `rust-json = { path = "../RS-JSON", features = ["fast"] }`。使用例は`RS-JSON/README.md`。

**このリポジトリでの適用状況**: 未適用(方針のみ)。コードの切り替えは、JSONを大きく・頻繁に読む所から実測して順に行う。

## JSON handling policy: use `parse_secure_sonic` (fast type + secure) and `sonic-rs` everywhere they can be used (2026-10-14, user instruction)
The source of truth is [`RS-JSON`](https://github.com/aon-co-jp/RS-JSON) (`README.md`, `PORTING.md`).

**Policy**: wherever JSON is read, use `parse_secure_sonic` from `rust-json` (RS-JSON, the `fast` feature; the result is `sonic_rs::Value`) **everywhere it can be used**.
Use `sonic-rs` directly where possible too (e.g. reading into typed structs). Where it cannot be used, use `parse_secure` (result: `serde_json::Value`).
Untrusted input from outside (HTTP bodies, files over the network, uploads) **must always be read with size / depth / duplicate-key limits**.

**Measured (7 MB, 40,000 records)**: `serde_json` 200 ms / `parse_secure` 217 ms / **`parse_secure_sonic` 83 ms (about 2.4x faster, with the defences)** / raw `sonic-rs` 33 ms (about 6x, no defences).
Reading into `serde_json::Value` gives almost no speed-up even with SIMD (188 ms). **The speed comes from the value type that packs the result into contiguous memory.**

**Where it can / cannot be used**
- ✅ Read-heavy code that parses large or frequent JSON (cache JSON, config, API responses, logs, indexes).
- ✅ Entrances for outside input (the goal there is defence rather than speed; `parse_secure` is fine).
- ⚠️ Existing code that passes `serde_json::Value` between functions needs its type changed to `sonic_rs::Value` (methods from `JsonValueTrait` / `JsonContainerTrait`). Reading into `serde_json::Value` without that change is not faster.
- ❌ WASM (`rust_json::light`, zero dependencies) and the JSON handling on the PHP / JavaScript side (Rust components cannot be used there).
- Note: the `sonic-rs` dependency uses `unsafe` internally (our own code has none). For small JSON (a few KB) the difference is not noticeable; do speed-motivated rewrites where measurements show a benefit.

**Adoption**: in `Cargo.toml`, `rust-json = { path = "../RS-JSON", features = ["fast"] }`. Example in `RS-JSON/README.md`.

**Status in this repository**: Not yet applied (policy only). Code will be switched over place by place, starting where measurements show large or frequent JSON reads.

## API 方針: REST 全廃・gRPC / Connect へ (2026-11-04)

### 日本語
- **REST は全廃する。新しい API は REST で作らない。** ブラウザや `curl` から呼ぶ API は **Connect の単純呼び出し**(`POST /<パッケージ>.<Service>/<Method>`、`content-type: application/json`、本文は JSON)、サービス間は **gRPC / gRPC-web**(RPoem の管理API `open_runo.admin.v1.Admin` と同じ流儀)。ストリーミングは、必要になってから。
- **既存の REST は、触るときに順に移す。** 移したら旧 REST の経路は消す。移行が終わるまで残るが、**新規には追加しない**。
- **JSON は RS-JSON に統一**する(読み込みは `from_str_secure` / `parse_secure` でサイズ・深さ・重複キーの上限つき、書き出しは `rust_json::fast`)。ブラウザの WASM に `prost` は入れない(サイズが増えるため)。
- **例外(自分たちの API ではなく、相手の規格が決めているもの)**: ACME、OAuth/OIDC のコールバック、第三者(GitHub など)からの Webhook、ヘルスチェック、静的ファイル配信、OTLP など。これらは規格どおりの HTTP を受ける。
- 参考実装: `RPoem`(`open_runo.admin.v1.Admin`)、`open-easy-web`(`server/src/connect.rs`、`proto/open_easy_web/admin/v1/admin.proto`)。
- ※ このリポジトリのコードが既に方針どおりかどうかは、各リポジトリの現状による。この節は方針の記録であり、コードの変更を意味しない。

### English
- **REST is being abolished. New APIs are not built as REST.** APIs called from browsers or `curl` use **Connect unary calls** (`POST /<package>.<Service>/<Method>`, `content-type: application/json`, JSON body); service-to-service calls use **gRPC / gRPC-web** (the same style as RPoem's admin API `open_runo.admin.v1.Admin`). Streaming only when it is actually needed.
- **Existing REST endpoints are migrated one by one, when they are touched.** The old REST route is removed once migrated. Until migration is finished they remain, but **no new ones are added**.
- **JSON is unified on RS-JSON**: reading with `from_str_secure` / `parse_secure` (size, depth and duplicate-key limits), writing with `rust_json::fast`. `prost` is not put into browser WASM (it would grow the binary).
- **Exceptions (shapes dictated by someone else's standard, not our own API)**: ACME, OAuth/OIDC callbacks, webhooks from third parties (e.g. GitHub), health checks, static file serving, OTLP, and so on. These accept plain HTTP as the standard requires.
- Reference implementations: `RPoem` (`open_runo.admin.v1.Admin`) and `open-easy-web` (`server/src/connect.rs`, `proto/open_easy_web/admin/v1/admin.proto`).
- Note: whether the code in a given repository already follows this policy depends on that repository. This section records the policy; it does not mean the code has been changed.

### Deutsch (KI-Übersetzung, redaktionell überarbeitet; noch nicht von Muttersprachlern geprüft)
- **REST wird vollständig abgeschafft; neue APIs entstehen nicht mehr als REST.** APIs, die aus dem Browser oder per `curl` aufgerufen werden, verwenden **unäre Connect-Aufrufe** (`POST /<Paket>.<Service>/<Methode>`, `content-type: application/json`, JSON-Body). Zwischen Diensten kommen **gRPC bzw. gRPC-web** zum Einsatz, im selben Stil wie die Admin-API von RPoem (`open_runo.admin.v1.Admin`). Streaming wird erst ergänzt, wenn es tatsächlich gebraucht wird.
- **Bestehende REST-Endpunkte werden schrittweise umgestellt, und zwar immer dann, wenn man ohnehin an ihnen arbeitet.** Sobald ein Endpunkt umgestellt ist, wird die alte REST-Route entfernt. Bis zum Abschluss der Migration bleiben die übrigen bestehen, es werden aber **keine neuen mehr hinzugefügt**.
- **JSON wird einheitlich über RS-JSON verarbeitet**: Gelesen wird mit `from_str_secure` bzw. `parse_secure` (mit Obergrenzen für Größe, Verschachtelungstiefe und doppelte Schlüssel), geschrieben mit `rust_json::fast`. `prost` wird nicht in das Browser-WASM aufgenommen, da es die Datei vergrößern würde.
- **Ausnahmen (Schnittstellen, deren Form ein fremder Standard vorgibt und die nicht zu unserer eigenen API gehören)**: ACME, OAuth/OIDC-Callbacks, Webhooks von Drittanbietern (z. B. GitHub), Health-Checks, die Auslieferung statischer Dateien, OTLP und Ähnliches. Diese nehmen, wie vom jeweiligen Standard verlangt, gewöhnliches HTTP entgegen.
- Referenzimplementierungen: `RPoem` (`open_runo.admin.v1.Admin`) und `open-easy-web` (`server/src/connect.rs`, `proto/open_easy_web/admin/v1/admin.proto`).
- Hinweis: Ob der Code eines einzelnen Repositorys diese Richtlinie bereits erfüllt, hängt vom jeweiligen Repository ab. Dieser Abschnitt dokumentiert lediglich die Richtlinie; er besagt nicht, dass der Code schon angepasst wurde.

### Русский (перевод выполнен с помощью ИИ и отредактирован; носителями языка пока не проверен)
- **От REST мы полностью отказываемся; новые API больше не создаются в стиле REST.** API, к которым обращаются из браузера или через `curl`, используют **унарные вызовы Connect** (`POST /<пакет>.<Service>/<Method>`, `content-type: application/json`, тело запроса — JSON). Для взаимодействия между сервисами применяются **gRPC и gRPC-web** — в том же стиле, что и административный API RPoem (`open_runo.admin.v1.Admin`). Потоковая передача данных добавляется лишь тогда, когда в ней действительно возникает необходимость.
- **Существующие REST-эндпоинты переводятся поэтапно — по мере того, как с ними приходится работать.** После перевода старый REST-маршрут удаляется. Пока миграция не завершена, оставшиеся эндпоинты сохраняются, но **новые не добавляются**.
- **Работа с JSON унифицирована на основе RS-JSON**: чтение — через `from_str_secure` или `parse_secure` (с ограничениями на размер, глубину вложенности и повторяющиеся ключи), запись — через `rust_json::fast`. `prost` в браузерный WASM не включается, поскольку это увеличило бы размер файла.
- **Исключения (интерфейсы, форму которых определяет чужой стандарт, а не наш собственный API)**: ACME, обратные вызовы OAuth/OIDC, вебхуки сторонних сервисов (например, GitHub), проверки работоспособности, раздача статических файлов, OTLP и тому подобное. Такие интерфейсы принимают обычный HTTP, как того требует соответствующий стандарт.
- Эталонные реализации: `RPoem` (`open_runo.admin.v1.Admin`) и `open-easy-web` (`server/src/connect.rs`, `proto/open_easy_web/admin/v1/admin.proto`).
- Примечание: соответствует ли код конкретного репозитория этой политике, зависит от самого репозитория. Данный раздел лишь фиксирует политику и не означает, что код уже изменён.

## API 方針の補足: 用語の整理と使い分け (2026-11-04)

### 日本語
**1つに統一するものではなく、別の層の話です。** 「REST の代わり」になるのは Connect・gRPC・GraphQL だけで、WASM は画面を作る技術です。

| 用語 | 何か | REST の代わりか |
|---|---|---|
| REST | URL と HTTP メソッドで操作を表す API の作り方 | 全廃する側 |
| **Connect** | `POST /サービス名/メソッド名` に JSON を送る形式。`curl` でもブラウザの `fetch` でも呼べる | はい(**採用**) |
| **gRPC** | 同じ「サービス/メソッド」の考え方を、バイナリ(protobuf)と HTTP/2 で行う形式。速く、ストリーミングも得意 | はい(**サービス同士に採用**) |
| gRPC-web | gRPC のブラウザ向けの変形版 | はい(必要なら) |
| GraphQL | 欲しい項目を問い合わせる形式 | 一部だけ(**問い合わせ(読み取り)専用**) |
| WASM | Rust をブラウザの中で動かす仕組み | いいえ(画面を作る技術) |

- **ブラウザの画面や `curl` から呼ぶ API は Connect**(POST、JSON を RS-JSON で読み書き)。ブラウザの WASM は素の gRPC を話せず、Connect なら `fetch` のままで、`prost` を足さずに済む(画面のサイズも増えない)。
- **サービス同士の通信(サーバー間、重い処理、ストリーミング)は gRPC。** 多くは Connect で足りるので、ストリーミングが必要になった所だけ gRPC に上げる。
- **GraphQL は、まとめて問い合わせたいときの読み取り専用。** 書き込み(操作)には使わない。
- **WASM は画面を作る技術として、そのまま使う。** 画面から呼ぶ API が Connect になっただけ。
- 外部の規格が形を決めるもの(ACME、OAuth のコールバック、第三者の Webhook、ヘルスチェック、静的ファイル、OTLP)は、規格どおりの HTTP のまま。
- **迷ったら Connect。**

### English
**These are not alternatives to unify; they sit at different layers.** Only Connect, gRPC and GraphQL can replace REST; WASM is a way to build the UI.

| Term | What it is | Replaces REST? |
|---|---|---|
| REST | API style that expresses operations through URLs and HTTP methods | The thing being abolished |
| **Connect** | `POST /Service/Method` with a JSON body; callable from `curl` and from a browser `fetch` | Yes (**adopted**) |
| **gRPC** | The same service/method idea over binary (protobuf) and HTTP/2; fast, good at streaming | Yes (**for service-to-service**) |
| gRPC-web | A browser-friendly variant of gRPC | Yes (if needed) |
| GraphQL | A query language: ask for exactly the fields you want | Partly (**read-only queries**) |
| WASM | Runs Rust inside the browser | No (a UI technology) |

- **APIs called from browser screens or `curl` use Connect** (POST, JSON read and written with RS-JSON). Browser WASM cannot speak plain gRPC; with Connect it stays a `fetch`, and `prost` is not needed (the binary does not grow).
- **Service-to-service calls (server to server, heavy work, streaming) use gRPC.** Connect is enough in most places; move a call to gRPC only where streaming is actually needed.
- **GraphQL is read-only**, for queries that gather several pieces of data. It is not used for writes (operations).
- **WASM stays as the way to build the UI.** The API it calls simply became Connect.
- Interfaces whose shape an outside standard dictates (ACME, OAuth callbacks, third-party webhooks, health checks, static files, OTLP) stay plain HTTP.
- **When in doubt, use Connect.**

### Deutsch (KI-Übersetzung, sprachlich überarbeitet; noch nicht von Muttersprachlern geprüft)
**Es geht nicht darum, eine einzige Technik auszuwählen – die Begriffe liegen auf unterschiedlichen Ebenen.** Als Ersatz für REST kommen nur Connect, gRPC und GraphQL in Frage; WASM dagegen ist eine Technik, mit der die Benutzeroberfläche gebaut wird.

| Begriff | Was ist das? | Ersatz für REST? |
|---|---|---|
| REST | Ein API-Stil, bei dem Operationen über URLs und HTTP-Methoden ausgedrückt werden | wird abgeschafft |
| **Connect** | `POST /Dienst/Methode` mit JSON im Body; aufrufbar mit `curl` ebenso wie mit `fetch` im Browser | ja (**gewählt**) |
| **gRPC** | Dasselbe Prinzip aus Dienst und Methode, aber über binäres protobuf und HTTP/2; schnell und gut für Streaming geeignet | ja (**zwischen Diensten**) |
| gRPC-web | Eine für den Browser angepasste Variante von gRPC | ja (bei Bedarf) |
| GraphQL | Abfragesprache, mit der man genau die benötigten Felder anfordert | teilweise (**nur für lesende Abfragen**) |
| WASM | Führt Rust-Code direkt im Browser aus | nein (Technik für die Oberfläche) |

- **APIs, die von Browseroberflächen oder per `curl` aufgerufen werden, verwenden Connect** (POST; JSON wird mit RS-JSON gelesen und geschrieben). WASM im Browser kann kein reines gRPC sprechen. Mit Connect bleibt es bei einem gewöhnlichen `fetch`, und `prost` ist nicht nötig – die WASM-Datei wird dadurch nicht größer.
- **Für die Kommunikation zwischen Diensten (Server zu Server, rechenintensive Aufgaben, Streaming) wird gRPC eingesetzt.** In den meisten Fällen reicht Connect aus; auf gRPC wird nur dort umgestellt, wo Streaming tatsächlich gebraucht wird.
- **GraphQL dient ausschließlich lesenden Abfragen**, wenn mehrere Informationen gebündelt abgefragt werden sollen. Für Schreibzugriffe (Operationen) wird es nicht verwendet.
- **WASM bleibt die Technik für die Oberfläche.** Geändert hat sich lediglich, dass die aufgerufene API nun auf Connect beruht.
- Schnittstellen, deren Form durch einen externen Standard festgelegt ist (ACME, OAuth-Callbacks, Webhooks von Drittanbietern, Health-Checks, statische Dateien, OTLP), bleiben gewöhnliches HTTP.
- **Im Zweifel: Connect.**

### Русский (перевод выполнен с помощью ИИ и литературно отредактирован; носителями языка пока не проверен)
**Это не варианты, из которых нужно выбрать один, а технологии разных уровней.** Заменить REST могут только Connect, gRPC и GraphQL; WASM же — это способ создания пользовательского интерфейса.

| Термин | Что это | Заменяет REST? |
|---|---|---|
| REST | Стиль построения API, при котором операции выражаются через URL и HTTP-методы | отменяется |
| **Connect** | `POST /Сервис/Метод` с телом в формате JSON; вызывается как из `curl`, так и из браузерного `fetch` | да (**выбран**) |
| **gRPC** | Тот же принцип «сервис — метод», но поверх двоичного формата protobuf и HTTP/2; работает быстро и хорошо подходит для потоковой передачи | да (**для связи между сервисами**) |
| gRPC-web | Адаптированный для браузера вариант gRPC | да (при необходимости) |
| GraphQL | Язык запросов: клиент запрашивает ровно те поля, которые ему нужны | частично (**только запросы на чтение**) |
| WASM | Позволяет выполнять код на Rust прямо в браузере | нет (технология для интерфейса) |

- **API, которые вызываются из браузерных интерфейсов или через `curl`, строятся на Connect** (POST; JSON читается и записывается через RS-JSON). Браузерный WASM не умеет работать с «чистым» gRPC. С Connect достаточно обычного `fetch`, а `prost` не требуется — размер WASM-файла не увеличивается.
- **Для обмена между сервисами (сервер — сервер, ресурсоёмкие задачи, потоковая передача) применяется gRPC.** В большинстве случаев достаточно Connect; на gRPC переходят только там, где потоковая передача действительно необходима.
- **GraphQL используется только для чтения**, когда нужно получить сразу несколько связанных данных. Для записи (операций) он не применяется.
- **WASM по-прежнему остаётся технологией для интерфейса.** Изменилось лишь то, что вызываемый им API теперь построен на Connect.
- Интерфейсы, форма которых задана внешним стандартом (ACME, обратные вызовы OAuth, вебхуки сторонних сервисов, проверки работоспособности, статические файлы, OTLP), остаются обычным HTTP.
- **Если есть сомнения — выбирайте Connect.**


## 2026-11-19 別アカウントでの再開用メモ(実測の到達点と再開用メッセージ) / Resume memo for another account

### 日本語
**リポジトリ**: [`aon-co-jp/realdata.pro`](https://github.com/aon-co-jp/realdata.pro)(`F:\realdata.pro`、公開 https://realdata.pro/)。関連: [`aruaru-search`](https://github.com/aon-co-jp/aruaru-search)(無料メタ検索)、`aruaru-llm`、非公開の保存先 `aon-co-jp/realdata-archive`(と`realdata-catalog`)。
**今回の到達点(`892e8e2`、push済み・VPS反映済み)**: 画面の「検索と毎朝の自動収集の状態」パネルの「直近の自動収集」が、サーバーを再起動しても消えないようにした。メモリに結果が無いとき、読み込み済みの保存分(`daily_jp_YYYYMMDD`)から復元する(成功・失敗の数は保存していないので0とし、保存先欄に「再起動前の記録。成功・失敗の数は不明」と出る)。`cargo test -p rrd-server crawl` 4件成功。本番は11/18 8:09から修正入りで稼働中、`crawlStatus` は 11/19 の `23か所成功・1か所失敗・保存先GitHub` を返すのを実測。
**毎朝の自動収集の実測(VPSログ)**: 11/14=23成功0失敗、11/15=22成功1失敗、11/17=23成功0失敗、11/18=23成功0失敗、11/19=23成功1失敗。失敗が出ても1か所で、「4か所続けて使えなかったときだけ打ち切る」修正(`MAX_UNAVAILABLE_STREAK`)が効いている。1失敗の原因は未調査。aruaru-search側の記録に残る失敗はBaiduの「取得できず」(ボット検知=回避しない方針で休止)。
**ドメイン**: `realdatapro.tokyo` は存在しない(DNSに無い)。公開名は `realdata.pro`。別ドメインを使うなら本人が正しい名前を決めてから設定する。
**未了**: (1)11/15・11/19の1失敗の原因調査(検索元の休止か、地域ごとの結果ゼロか)、(2)数日分の結果を見た検索元ごとの失敗傾向の分析、(3)Naver/Baiduの品質調整、(4)ja/en/zh以外の言語のリサーチ、(5)各社の無料枠を毎朝確認する機能(依頼済み・未着手)、(6)aruaru-searchの公開ドメイン(未決・既定は公開しない)。
**注意**: `cargo fmt --all` は使わない(兄弟リポジトリまで整形される。`cargo fmt-own`/`lint-own`/`test-own`)。ローカルで`cargo`を動かす前に`scripts\fetch-deps.ps1`で`.deps/`を取得する(`deps.lock`で固定)。PowerShellでは`$env:PATH="$env:USERPROFILE\.cargo\bin;$env:PATH"`が必要。トークンは`F:\git.txt`等のtxt経由で、値は表示・コミットしない。VPS反映は `ssh conoha` → `/root/repository/realdata.pro` で `git pull --ff-only` → `cargo build --release -j 2 -p rrd-server` → **自動収集が走っていない時間に**(毎朝7時開始で約3.5時間)`systemctl restart realdata-pro`。
**再開用メッセージ(別アカウントでそのまま貼り付け)**: 「`aon-co-jp/realdata.pro`(`F:\realdata.pro`)の続きです。`PORTING.md` 末尾の『2026-11-19 別アカウントでの再開用メモ』を読んでから再開してください。直近の自動収集の復元(`892e8e2`)は本番反映済みです。次は、(1)11/15・11/19の自動収集で1か所失敗した原因の調査、(2)数日分の結果から検索元ごとの失敗の傾向の分析、(3)ドメインは `realdata.pro` のまま(別名にするなら先に確認)。返答は日本語のみ、REST は作らず Connect のみ、秘密は表示せず、完了報告の前に本番で確認、pushやVPS再起動は確認してから(自動収集の実行中は再起動しない)。」

### English
**Repositories**: [`aon-co-jp/realdata.pro`](https://github.com/aon-co-jp/realdata.pro) (`F:\realdata.pro`, public https://realdata.pro/). Related: [`aruaru-search`](https://github.com/aon-co-jp/aruaru-search) (free meta search), `aruaru-llm`, and the private stores `aon-co-jp/realdata-archive` (and `realdata-catalog`).
**Reached this time (`892e8e2`, pushed and deployed)**: the "last daily crawl" in the "search and daily crawl status" panel no longer disappears when the server restarts. When memory has no result, it is restored from the already loaded saved data (`daily_jp_YYYYMMDD`); the success/failure counts are not stored, so they show 0 and the storage column reads "record from before restart; success/failure counts unknown". `cargo test -p rrd-server crawl` passes (4 tests). Production has run the fixed build since 11-18 08:09, and `crawlStatus` was measured returning 11-19: 23 succeeded, 1 failed, saved to GitHub.
**Measured daily crawls (VPS log)**: 11-14 = 23 ok / 0 failed, 11-15 = 22 / 1, 11-17 = 23 / 0, 11-18 = 23 / 0, 11-19 = 23 / 1. Even when a failure occurs it is one place, so the "stop only after 4 consecutive unavailable" fix (`MAX_UNAVAILABLE_STREAK`) works. The cause of the single failures is not yet investigated. The only failure recorded on the aruaru-search side is Baidu "could not fetch" (bot detection; by policy we do not bypass it and let it rest).
**Domain**: `realdatapro.tokyo` does not exist (no DNS record). The public name is `realdata.pro`. If another domain is wanted, the owner must decide the real name first.
**Open**: (1) find the cause of the one failure on 11-15 and 11-19 (a resting search source, or zero results for a region), (2) analyse failure trends per search source over several days, (3) tune Naver/Baidu quality, (4) research in languages other than ja/en/zh, (5) the requested daily check of each provider's free tier (not started), (6) a public domain for aruaru-search (undecided; the default is not to publish).
**Cautions**: never run `cargo fmt --all` (it formats sibling repositories; use `cargo fmt-own`/`lint-own`/`test-own`). Before running `cargo` locally, fetch `.deps/` with `scripts\fetch-deps.ps1` (pinned by `deps.lock`). In PowerShell set `$env:PATH="$env:USERPROFILE\.cargo\bin;$env:PATH"`. Tokens go through txt files such as `F:\git.txt`; never print or commit values. VPS deploy: `ssh conoha` → in `/root/repository/realdata.pro` run `git pull --ff-only` → `cargo build --release -j 2 -p rrd-server` → `systemctl restart realdata-pro` **only when no daily crawl is running** (it starts at 07:00 and takes about 3.5 hours).
**Resume message (paste as is on the other account)**: "Continuing `aon-co-jp/realdata.pro` (`F:\realdata.pro`). Read 'Resume memo for another account (2026-11-19)' at the end of `PORTING.md` first. Restoring the last daily crawl after restarts (`892e8e2`) is deployed to production. Next: (1) investigate why one place failed in the 11-15 and 11-19 crawls, (2) analyse per-source failure trends over several days, (3) keep the domain `realdata.pro` (ask before choosing another). Reply in Japanese only, build no REST (Connect only), never print secrets, verify in production before reporting done, confirm before pushing or restarting the VPS (never restart while a daily crawl is running)."

## Versionless API の方針 / Versionless API policy (2026-11-25)
**日本語**: このリポジトリのAPIは **Versionless**(URL・パッケージ名に `v1` 等を付けず、互換性を壊さず足すだけで進化させる)。欄は足すだけ・足す欄は省略可能・知らない欄は無視・名前変更は新名を足して旧を非推奨に(期限は既定で6か月以上、使用0を確認してから削除)・enumは未知の値を許容・安全上の緊急修正は例外。既存の `.v1` は呼び出し側を壊さないよう当面残し、版なしの別名を追加して段階的に畳む。詳細・出典は [`runo/API-POLICY.md`](https://github.com/aon-co-jp/runo/blob/main/API-POLICY.md) 末尾。適用はこれから順に行う。
**English**: This repository's APIs are **Versionless**: no `v1` in URLs or package names; evolve by adding only, without breaking compatibility. Only add fields, new fields are optional, ignore unknown fields, rename by adding the new name and deprecating the old (default deadline 6 months or more, remove only after confirming zero use), tolerate unknown enum values, emergency security fixes are the exception. Existing `.v1` names stay for now so callers do not break; a versionless alias is added and `.v1` is retired in stages. Details and sources: the end of [`runo/API-POLICY.md`](https://github.com/aon-co-jp/runo/blob/main/API-POLICY.md). Rollout is step by step.
