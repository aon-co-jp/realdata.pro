# 開発方針(rs-real-data)

作業ドライブは `F:\rs-real-data`。全リポジトリ共通ルールは
[`open-raid-z/CLAUDE.md`](https://github.com/aon-co-jp/open-raid-z) を正本として参照すること。

## このリポジトリの役割

SAS Viyaを参考にした、Rust製オープンソースのインメモリ分析プラットフォーム。
公式ドメインは `realdata.pro`(ConoHa DNS: ns-a1/a2/a3.conoha.io、VPS 160.251.237.162 を予定)。

- `rrd-core` は外部クレートに依存させない。
- SAS社のコード・API・言語仕様を複製しない。商標の注意書きをREADMEに残す。
- 計画と進捗は [PORTING.md](PORTING.md) で管理する。

## ビルドとテスト

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

## HANDOFF

- **2026-09-24 新規作成**: P0(コアエンジンとCLI)を実装し、テスト9件がすべて通過。
  サンプルCSVでCLIの全サブコマンドの動作を確認済み。
  次はP1(日付型・join・Parquet)、またはrealdata.proでのWeb UI(P3)。
  realdata.proは作成時点でDNSの委任が未完了(レジストリにNS未登録、ConoHa DNSにゾーン未作成)。
