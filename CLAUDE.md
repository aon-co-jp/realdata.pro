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
