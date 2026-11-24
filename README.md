> Latest translations / 翻訳の最新版: [0-LATEST-TRANSLATIONS.md](0-LATEST-TRANSLATIONS.md)

# realdata.pro

**誰でも使える、Rust製・オープンソースのデータ分析プラットフォーム**(SAS Viyaの考え方を参考にした自由実装)。
大企業にも、中小企業にも、これから起業する人にも。CSVをドラッグするだけで、クレンジング・集計・予測まで。

公式サイト: https://realdata.pro

> SAS / SAS Viya は SAS Institute Inc. の商標です。本プロジェクトは SAS Institute とは無関係で、
> 同社のコード・API・仕様を複製するものではありません。機能の方向性のみを参考にしています。

## やさしい説明 / Plain-language guide

### 日本語
**realdata.pro は、データを集めて、整理して、グラフや文章で分かりやすく見せてくれる、無料の道具です。**

- **なにができるの?** 表(CSVファイル)をドラッグするだけで、集計・グラフ・予測ができます。むずかしい知識はいりません。
- **世界リサーチ**: 「調べたいこと」と「国や地域」を選ぶと、そこの言葉でニュースやWebを探して、AIが「いま何が起きているか」「チャンスは何か」「気をつけることは何か」をまとめます。どの話も、元の記事へのリンクが付いています。
- **毎朝の自動収集**: 毎朝7時(日本時間)に、日本じゅうの23か所について、観光・仕事・工場見学などの情報を自分で集めて、GitHubの非公開の保管庫に保存します。人が何もしなくても続きます。
- **お金がかかりません**: 有料の検索サービスは使いません。無料で動く自前の検索(`aruaru-search`)と、無料のAI(`aruaru-llm`)を使います。
- **公開サイト**: https://realdata.pro/ (いま動いています)。

**うまくいかなかった日は?** 検索サービスが「使いすぎです」と止めることがあります。そのときは、むりに通り抜けず、少し休んでから自動でもどります。「その場所に該当する情報がなかった」場合は、失敗ではなく「該当なし」と数えます。

**投資のアドバイスではありません。** 画面にある金融のサンプルは、練習用のシミュレーションです。

### English (simple and friendly)
**realdata.pro is a free tool. It collects data, tidies it up, and shows it to you as charts and easy words.**

- **What can it do?** Drop a table (a CSV file) on the page. It can add things up, draw charts, and make guesses about the future. You do not need hard skills.
- **World research**: Pick a topic and a country or region. The tool looks for news and web pages in the local language. Then an AI writes what is happening, where the chances are, and what to watch out for. Every point has a link to the real article.
- **Daily auto-collection**: Every morning at 7:00 (Japan time) it gathers information about 23 places in Japan, such as sightseeing, jobs, and factory tours. It saves everything in a private GitHub vault. It keeps going without anyone pressing a button.
- **It costs nothing**: It does not use paid search services. It uses its own free search (`aruaru-search`) and a free AI (`aruaru-llm`).
- **The public site**: https://realdata.pro/ (it is running now).

**What if a day goes wrong?** A search service may say, "You asked too much, please wait." The tool does not sneak past this. It rests for a while and then comes back by itself. If a place simply has no matching information, we count it as "nothing found", not as a failure.

**This is not investment advice.** The money screens are practice simulations.


## 🌍 世界リサーチ

テーマ(例:「抹茶の海外需要」「電気自動車の充電インフラ」)と国・地域を選ぶだけで、次のことを自動で行います。

1. 各国の現地語で Web とニュースを集める
2. テーマと無関係な情報を除く
3. AI(aruaru-llm の無料 AI)が、トレンド・機会・リスク・具体的な提案・国別の論調をまとめる

根拠はすべて、実際に集めた記事へのリンクで示します。レポートは日本語・英語と約130言語から選べます。
集めた記事はデータセットになり、集計やグラフにもそのまま使えます。

> **毎朝の自動収集の試験(2026-09-26)**: 日本全国48か所を対象に試験中。1か所目(岩手県)は27件の検索で
> 70件を収集(以前は0件で失敗)。既定は1日600件(約22か所/日)で、全国一巡に約2日の見込み。詳細は
> [PORTING.md](PORTING.md)・[CLAUDE.md](CLAUDE.md)を参照。
>
> *Automatic daily collection test (2026-09-26, English): testing across 48 locations in Japan. The first
> location (Iwate) collected 70 records from 27 searches (previously failed with 0). Default is 600
> searches/day (~22 locations/day), an estimated ~2 days for a full national pass. See
> [PORTING.md](PORTING.md) / [CLAUDE.md](CLAUDE.md) for details.*

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
| グラフ描画 | 棒・円グラフ(ブラウザ SVG)と、[open-directx](https://github.com/aon-co-jp/open-directx)(Vulkan)による GPU 描画。GPU が無ければ AVX-512 / AVX2 の CPU 描画 | ✅ |
| データ保管庫(GitHub 非公開リポジトリ)の容量管理 | 1リポジトリ10GB上限に備え、索引リポジトリ設定時は容量が近づくと新しいリポジトリへ自動で引っ越す | ✅ 2026-09-28 |

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

## 保管庫のお引越し(自動)

収集データは GitHub の非公開リポジトリ(保管庫)に保存します。1リポジトリが約1GBに近づく(増加ペースから14日以内と予測した)と、
新しい非公開リポジトリ(`realdata-archive-2` …)を自動で作り、書き込み先を切り替えます。古いデータは移動せず、読み取り専用のまま残ります。
索引リポジトリ `RRD_CATALOG_REPO` を設定すると有効になります(未設定なら単一リポジトリ)。詳細は [PORTING.md](PORTING.md)。2026-11-08に使い捨てリポジトリで実地確認済み(新シャード自動作成・旧シャードの読み取り専用化・索引の切替)。

*English: when one vault repository nears ~1 GB the server auto-creates the next private repository and switches writes; old shards stay read-only. Verified live on 2026-11-08 with throwaway repositories. See [PORTING.md](PORTING.md). To resume work on another account (repository: `realdata.pro`), see the resume message in [CLAUDE.md](CLAUDE.md) (updated 2026-11-17).*

## 前面(open-web-server)・アプリサーバー(RPoem)との関係

前面は `open-web-server`(Apache相当)、アプリサーバーは `RPoem`(Tomcat相当)。両者の「4層4重」通信・DB連携の定義(通信の四重化・DB書き込みの四重化など)と、realdata.pro がその書き込み経路を**まだ使っていない**という現状は [CLAUDE.md](CLAUDE.md)・[PORTING.md](PORTING.md) に日英で記録した。

*English: the front is `open-web-server` (Apache-like) and the app server is `RPoem` (Tomcat-like). The definition of their "4-layer, 4-fold" communication and DB integration, and the honest status that realdata.pro does **not yet** use that write path, are in [CLAUDE.md](CLAUDE.md) and [PORTING.md](PORTING.md) (JA/EN).*

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

**Automatic archive rotation ("hikkoshi").** Collected data is stored in private GitHub repositories. When the current
shard nears ~1GB (or is predicted to within 14 days), a new private repository (`realdata-archive-2`, …) is created
automatically and writes switch to it; old shards stay read-only. Enabled by setting the catalog repository
`RRD_CATALOG_REPO`; otherwise a single repository is used. Details in [PORTING.md](PORTING.md).

## ライセンス

MIT OR Apache-2.0([LICENSE-MIT](LICENSE-MIT) / [LICENSE-APACHE](LICENSE-APACHE))

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

---

作者 / Author: 石 塚 正 浩（MASAHIRO ISHIZUKA） (https://github.com/aon-co-jp)

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


## 引っ越し・再開 / Resume
最新の到達点と再開用メッセージ(日英)は [`PORTING.md`](PORTING.md) 末尾「2026-11-19 別アカウントでの再開用メモ」。 / The latest status and a paste-ready resume message (JA/EN) are at the end of [`PORTING.md`](PORTING.md) ("Resume memo for another account", 2026-11-19).
