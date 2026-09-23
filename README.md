# rs-real-data

**Rust製・オープンソースのインメモリ分析プラットフォーム**(SAS Viyaの主要な考え方を参考にした自由実装)。
公式サイト(予定): https://realdata.pro

> SAS / SAS Viya は SAS Institute Inc. の商標です。本プロジェクトは SAS Institute とは無関係で、
> 同社のコード・API・仕様を複製するものではありません。機能の方向性のみを参考にしています。

## 目指すもの

| SAS Viyaの特徴 | rs-real-data での対応 | 状態 |
|---|---|---|
| インメモリ分散処理エンジン(CAS) | 列指向インメモリ DataFrame (`rrd-core`) | ✅ 単一ノード版 |
| データの準備・クレンジング | 欠損補完・欠損行/重複行の除去・フィルタ・並べ替え・列選択 | ✅ |
| 探索的データ分析 | `describe`(件数・欠損数・平均・標準偏差・四分位・最小/最大)・相関 | ✅ |
| 集計 | group by + count/sum/mean/min/max/median/std | ✅ |
| 予測・機械学習 | 単回帰 | ✅ 最小限(重回帰・決定木等は今後) |
| ノーコード/ローコードのGUI | Web UI | 🔜 計画中 |
| Python / R 連携 | PyO3 バインディング(Python)・R バインディング | 🔜 計画中 |
| クラウド/オンプレ両対応 | 単一バイナリ・依存クレートなし | ✅(分散化は今後) |

詳細な設計と今後の計画は [PORTING.md](PORTING.md) を参照。

## 構成

```
crates/
  rrd-core/   コアエンジン(外部依存なし): CSV読込/書出・型推論・DataFrame・統計
  rrd-cli/    コマンドライン `rrd`
examples/
  sales.csv   サンプルデータ
```

## 使い方

```bash
cargo build --release
rrd describe examples/sales.csv
rrd query examples/sales.csv --dedup --fill sales:mean --filter "sales>=60000" --sort sales:desc
rrd groupby examples/sales.csv city sales:sum sales:count qty:mean
rrd corr examples/sales.csv qty sales
rrd regress examples/sales.csv ad_cost sales
```

出力例(`groupby`):

```
| city    | sales_sum | sales_count | qty_mean |
|---------|-----------|-------------|----------|
| Tokyo   | 450000    | 4           | 10       |
| Osaka   | 250000    | 3           | 8.3333   |
| Fukuoka | 75000     | 2           | 3.5      |
```

## ライブラリとして

```rust
use rrd_core::{csv, Agg, Fill};

let df = csv::read_csv("examples/sales.csv")?
    .drop_duplicates()
    .fill_null("sales", &Fill::Mean)?;
println!("{}", df.group_by("city", &[("sales", Agg::Sum)])?);
```

## English

**rs-real-data** is an open-source, in-memory analytics platform written in Rust, inspired by the
concepts of SAS Viya (not affiliated with SAS Institute). The current release provides a
dependency-free columnar DataFrame engine (`rrd-core`) with CSV I/O and type inference, data
cleansing (null filling, dropping nulls/duplicates, filtering, sorting), exploratory statistics
(`describe`, correlation), group-by aggregation and simple linear regression, plus the `rrd` CLI.
A web UI, Python/R bindings and distributed execution are planned — see [PORTING.md](PORTING.md).

## ライセンス

MIT OR Apache-2.0([LICENSE-MIT](LICENSE-MIT) / [LICENSE-APACHE](LICENSE-APACHE))
