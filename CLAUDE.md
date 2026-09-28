# 開発方針(realdata.pro)

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
