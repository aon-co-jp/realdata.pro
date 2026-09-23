# 開発方針(rs-real-data)

作業ドライブは `F:\rs-real-data`。全リポジトリ共通ルールは
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
- realdata.pro は、作成時点で DNS の委任が完了していない(レジストリに NS が未登録、ConoHa DNS にゾーンが未作成)。
- `.github/workflows/ci.yml` は、gh トークンに `workflow` 権限が付くまで push できない。
