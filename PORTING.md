# PORTING / 設計と計画

## 設計思想

1. **誰でも使える**: 大企業・中小企業・これから起業する人の誰もが、専門知識なしで使えること。
   - ファイルを選ぶだけで始められる。操作は日本語で案内し、エラーは「何をすればよいか」を書く。
   - 単一バイナリで、インストールも保守も簡単にする。無料のOSSとして提供し、realdata.pro でホスト版も用意する。
   - 同じ機能を GraphQL でも提供し、社内システムや Python/R からも使えるようにする。
2. **エコシステムで作る**: Rust を基盤に、open-web-server・RPoem・open-cuda・aruaru-db・aruaru-llm・
   open-directx の上に組み立てる。同じ機能をこのリポジトリで重ねて実装しない。
3. **周りを壊さない(2026-09-24 追加)**: ビルド・検査・整形が、ユーザーの作業ツリーや他のリポジトリを
   一切変更しないこと。
   - 依存リポジトリは `deps.lock` でコミットを固定し、`scripts/fetch-deps` で `.deps/`(Git 管理外)へ取得する。
     `F:\` 直下の作業ツリーは参照しない。
   - 整形・検査は自リポジトリのクレートに限定する(`cargo fmt-own` / `lint-own` / `test-own`)。
   - CI の最後に「`.deps/` が一切変更されていない」ことを検査する。
   - `.deps/` に手作業の変更があれば、`fetch-deps` は上書きせずに停止する。
   - 経緯: `cargo fmt --all` がパス依存先の兄弟リポジトリ約200ファイルを整形してしまう事故が起きた。
     HEAD を rustfmt した結果と一致するものだけを戻して復旧した。
4. **REST を増やさない**: エコシステムの方針に従い、操作は `POST /graphql` 1本に集約する。
5. **SAS 社の資産を複製しない**: 機能の方向性のみを参考にし、コード・API・言語仕様は複製しない。

## フェーズ

| フェーズ | 内容 | 状態 |
|---|---|---|
| P0 | 列指向 DataFrame・CSV 入出力・型推論・クレンジング・要約統計・group by・相関・単回帰・CLI | ✅ 2026-09-24 |
| P1 | open-cuda で重回帰、RPoem + GraphQL サーバー、ノーコード Web UI、依存の固定取得 | ✅ 2026-09-24 |
| P1.5 | 取り込み元の追加: 検索ワード(Google / YouTube / GitHub、aruaru-llm `POST /v1/search/raw` 経由)、調査対象 URL(CSV・JSON・HTML の表・リンク一覧、SSRF 対策付き) | ✅ 2026-09-24 |
| P3 | aruaru-llm 連携: 分析結果を日本語・英語と、約130言語から選んだ1言語で説明する(同意必須、生データは既定で送らない) | ✅ 2026-09-24 |
| P2 | aruaru-db 連携: データセットの保存・読込、`commit()` で分析の版を記録し、`AS OF COMMIT` で再現する | 次 |
| P4 | open-directx 連携: GPU(Vulkan)でグラフ(棒・円)を PNG に描く。GPU が無い環境では、open-cpu の実行時判定で AVX-512(16画素並列)/ AVX2(8画素並列)/ スカラーの CPU ラスタライザで描く(`rrd-render`、GraphQL `renderChart`) | ✅ 2026-09-24(デスクトップビューアは未着手) |
| P5 | realdata.pro 公開: open-web-server を前段(TLS/ACME・ドメイン振り分け)にして VPS で運用する | |
| P6 | 日付型・join・ピボット・Parquet、ロジスティック回帰・決定木・k-means | |
| P7 | GPU バックエンド(Vulkan/DirectX)での GEMM、複数ノード分散 | |

## グラフ描画(P4、2026-09-24)

- グラフは三角形メッシュにしてから描く(棒: 背景・目盛り線・軸・棒の四角形。円: 1度あたり1分割以上の扇形)。
  縦横2倍で描いてから 2×2 平均で縮小し、ジャギーを抑える。文字(ラベル・凡例)は画像に含めず、画面側の HTML で出す。
- **GPU**: open-directx `render_indexed_scene_with_depth_and_read_back`(Vulkan オフスクリーン)を使う。
  シェーダーは open-directx 同梱の `triangle_vs/ps.dxbc` を `directx-shader-translate` で SPIR-V に変換して使う。
  深度で「後に描いた三角形が手前」になるようにし、CPU 経路の後勝ちと同じ見え方にしている。
- **CPU**: エッジ関数によるラスタライザ。スカラー版と SIMD 版で計算の順序と丸めを揃えている
  (行定数 `B·y+C` を先に求め、FMA は使わない)。
- 検証結果(2026-09-24):
  - 開発機(Ryzen 9 3950X / GeForce GT 730): GPU と CPU の出力が**完全一致**(約28万画素中、差は0画素)。
    AVX2 とスカラーの出力も画素単位で一致した。
  - AVX-512 の経路は VPS(Xeon Icelake)で検証する(下の HANDOFF 参照)。
- `RRD_SIMD=avx2|scalar` で下位の経路に固定できる(比較・切り分け用)。

## API キーの共有(2026-09-24)

検索(Google / YouTube / GitHub)と AI のキーは、**aruaru-llm の設定1か所を正本**とし、realdata.pro は
キーを持たない(コピーしない)。realdata.pro は `RRD_ARUARU_LLM_URL` で aruaru-llm を呼ぶだけにする。

- VPS では、open-english と realdata.pro が同じ aruaru-llm(`127.0.0.1:4600`)を使う。
  キーは `/root/aruaru-llm/.env.google-search` などにある。
  ここへキーを追加・変更すると、両方に自動で即時反映される(コピーしないので、片方だけ古いままになるずれも起きない)。
- open-english の画面で利用者がブラウザに保存するキー(localStorage / vault)は、その利用者のブラウザにだけ存在する。
  別オリジン(realdata.pro)からは読み取れないため、自動共有の対象外。
- 2026-09-24 時点のローカル検証: 開発機のキー(`F:\API.txt`)では、Google Custom Search JSON API が
  「このプロジェクトは API へのアクセス権がない」(HTTP 403)を返した。VPS の設定での動作確認は、デプロイ後に行う。

## 実装メモ

- `Column` は型ごとの `Vec<Option<T>>`(int/float/bool/str)。`None` が欠損。
- CSV の型推論は int → float → bool → str の順。欠損とみなす値: 空欄 / `NA` / `N/A` / `null` / `NULL` / `NaN`。
  3桁区切りのカンマ(`8,021,407,192`)は数値として扱う(区切り位置が不正なものは文字列のまま)。
- URL 取り込みの SSRF 対策: 許可するのは http/https のみ。名前解決したすべての IP を検査し、1つでも内部向けなら拒否する
  (私設・ループバック・リンクローカル・CGNAT・NAT64 等)。接続は検証済みの IP へ固定する。
  リダイレクトは手動で追い、1段ごとに再検証する(最大5回)。応答は 16MiB・20秒まで。
- AI 説明に渡すのは、要約統計と、画面に出ている分析結果の文字だけ。生データは、利用者が選んだ場合に限り先頭10行を渡す。
  AI の回答は、innerHTML を使わずに要素を組み立てて表示する(見出し・太字・箇条書きのみ対応)。
- 数値列を平均値・中央値で補完すると float 列になる。フィルタでは欠損の行を常に除外する。
- 重回帰: 説明変数と目的変数を平均で中心化した拡大行列 A について、AᵀA を open-cuda の `sgemm`(f32)で求める。
  XᵀX と Xᵀy はそこから取り出し、部分ピボット付きガウス消去(f64)で解く。R² は元データから f64 で計算する。
  2026-09-24 に厳密な有理数計算と照合し、係数の相対誤差は 2×10⁻⁵ 程度だった(f32 GEMM による)。
- サーバーはリクエストボディの上限(`RRD_MAX_BODY`、既定 32MiB)を、読み込みの時点で適用する。
  データセットは最大64個までとし、GraphQL には深さ・複雑度の上限を設けている。
- 現在、データセットはメモリ上にのみ保持している(再起動で消える)。永続化は P2 の aruaru-db で行う。

## 普及計画

[docs/GO_TO_MARKET.md](docs/GO_TO_MARKET.md) を参照。
