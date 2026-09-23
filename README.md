# realdata.pro

**誰でも使える、Rust製・オープンソースのデータ分析プラットフォーム**(SAS Viyaの考え方を参考にした自由実装)。
大企業にも、中小企業にも、これから起業する人にも。CSVをドラッグするだけで、クレンジング・集計・予測まで。

公式サイト(準備中): https://realdata.pro

> SAS / SAS Viya は SAS Institute Inc. の商標です。本プロジェクトは SAS Institute とは無関係で、
> 同社のコード・API・仕様を複製するものではありません。機能の方向性のみを参考にしています。

## aon-co-jp エコシステムで構成

| 役割 | 使うもの | 状態 |
|---|---|---|
| 分析エンジン(列指向インメモリ DataFrame) | `rrd-core`(本リポジトリ、外部依存なし) | ✅ |
| 行列演算(重回帰など) | [open-cuda](https://github.com/aon-co-jp/open-cuda) `opencuda-blas` の GEMM | ✅ CPU バックエンドで実働 |
| Web サーバー・API | [RPoem](https://github.com/aon-co-jp/RPoem) `open-runo-poem-compat` + GraphQL 単一エンドポイント | ✅ |
| ネットワーク基盤 | [open-web-server](https://github.com/aon-co-jp/open-web-server)(RPoem 経由で組込み)、本番の前段ゲートウェイ | ✅ 組込み / 🔜 realdata.pro 前段 |
| データの永続化・版管理 | [aruaru-db](https://github.com/aon-co-jp/aruaru-db)(Git-on-SQL、`AS OF COMMIT` で過去の分析を再現) | 🔜 次段 |
| 検索ワードからの取り込み(Google / YouTube / GitHub) | [aruaru-llm](https://github.com/aon-co-jp/aruaru-llm) `POST /v1/search/raw` | ✅ |
| 調査対象 URL からの取り込み(CSV・JSON・HTML の表) | 本リポジトリ(SSRF 対策付き) | ✅ |
| AI による分析の説明(日本語・英語と約130言語から選択) | [aruaru-llm](https://github.com/aon-co-jp/aruaru-llm) | ✅ |
| グラフ描画 | 棒グラフ・円グラフ(ブラウザ SVG)/ GPU 描画は [open-directx](https://github.com/aon-co-jp/open-directx) | ✅ 棒・円 / 🔜 GPU |

## SAS Viya の特徴との対応

| SAS Viyaの特徴 | realdata.pro | 状態 |
|---|---|---|
| インメモリ分散処理エンジン(CAS) | 列指向 DataFrame + open-cuda | ✅ 単一ノード |
| データ準備・クレンジング | 欠損補完・欠損行/重複行除去・フィルタ・並べ替え・列選択 | ✅ |
| 探索的データ分析 | 要約統計(四分位など)・相関 | ✅ |
| 集計 | group by(count/sum/mean/min/max/median/std) | ✅ |
| 予測・機械学習 | 重回帰(open-cuda GEMM) | ✅ 最初の一歩 |
| ノーコード GUI | Web UI(ファイル選択だけで使える) | ✅ |
| Python / R 連携 | GraphQL で任意の言語から / 専用バインディング | ✅ GraphQL / 🔜 専用 |

## すぐ試す

```bash
# 1. エコシステムの依存をコミット固定で .deps/ に取得(作業ツリーには触れない)
bash scripts/fetch-deps.sh          # Windows: powershell -File scripts\fetch-deps.ps1
# 2. サーバー起動 → ブラウザで http://127.0.0.1:4701/
cargo run --release -p rrd-server
```

コマンドラインだけで使う場合:

```bash
cargo run --release -p rrd-cli -- describe examples/sales.csv
cargo run --release -p rrd-cli -- groupby examples/sales.csv city sales:sum qty:mean
```

GraphQL(`POST /graphql`)の例:

```graphql
mutation { loadCsv(name: "sales", csv: "city,sales\nTokyo,100\nOsaka,80\n") { rows } }
query { regression(name: "sales", target: "sales", features: ["qty", "ad_cost"]) { intercept coefficients rSquared device } }
```

## 構成

```
crates/
  rrd-core/     分析エンジン(外部依存なし)
  rrd-compute/  open-cuda による行列演算
  rrd-server/   RPoem 上の GraphQL + Web UI
  rrd-cli/      コマンドライン `rrd`
deps.lock       依存するエコシステムのリポジトリとコミット
scripts/        fetch-deps(.deps/ への固定取得)
```

開発では `cargo fmt-own` / `cargo lint-own` / `cargo test-own` を使います(`cargo fmt --all` は依存先まで整形するため使いません)。

## English

**realdata.pro** is an open-source analytics platform in Rust, inspired by SAS Viya (not affiliated with
SAS Institute), built on the aon-co-jp ecosystem: a dependency-free columnar engine, open-cuda GEMM for
regression, and a RPoem-based server exposing a single GraphQL endpoint plus a no-code web UI. aruaru-db
persistence, aruaru-llm explanations and open-directx rendering are next. See [PORTING.md](PORTING.md).

## ライセンス

MIT OR Apache-2.0([LICENSE-MIT](LICENSE-MIT) / [LICENSE-APACHE](LICENSE-APACHE))
