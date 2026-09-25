//! rrd-server: realdata.pro の Web サーバー。
//!
//! RPoem(`open-runo-poem-compat`、tokio/hyper 自前実装)の上で次を提供する。
//! 行列演算は open-cuda(`rrd-compute`)で実行する。
//!
//! - `GET  /`         Web UI(ノーコードで読込・クレンジング・探索・集計・回帰)
//! - `POST /graphql`  すべての操作の単一エンドポイント(async-graphql)
//! - `GET  /healthz`  ヘルスチェック
//!
//! 環境変数:
//! - `RRD_BIND`      待受アドレス(既定 127.0.0.1:4701)
//! - `RRD_MAX_BODY`  リクエストボディ上限バイト数(既定 32MiB)
//! - `RRD_ARCHIVE_REPO`  保存先の GitHub 非公開リポジトリ(設定すると保存・版管理が使える。認証は git の設定に任せる)
//! - `RRD_DATA_DIR`  収集結果の保存先(既定 data)
//! - `RRD_ARUARU_LLM_URL`  aruaru-llm の URL(既定 http://127.0.0.1:4600、検索取り込みと AI 説明に使う)

mod archive;
mod crawl;
mod deposits;
mod explain;
mod ingest;
mod languages;
mod market;
mod osm;
mod places;
mod regions;
mod research;
mod schema;
mod store;
mod tuning;

use std::net::SocketAddr;
use std::sync::Arc;

use http_body_util::{BodyExt, Limited};
use open_runo_poem_compat::hyper_compat::{html_response, json_response};
use open_runo_poem_compat::{
    get, handler_fn, post, Request, Response, Route, Server, StatusCode, TcpListener,
};
use schema::{build_schema, AppState, RrdSchema};

const INDEX_HTML: &str = include_str!("../web/index.html");

async fn graphql(schema: RrdSchema, req: Request, max_body: usize) -> Response {
    let bytes = match Limited::new(req.into_body(), max_body).collect().await {
        Ok(c) => c.to_bytes(),
        Err(_) => {
            return json_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                &serde_json::json!({ "errors": [{ "message": format!("リクエストが大きすぎます(上限 {max_body} バイト)") }] }),
            )
        }
    };
    let gql_req: async_graphql::Request = match serde_json::from_slice(&bytes) {
        Ok(r) => r,
        Err(e) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                &serde_json::json!({ "errors": [{ "message": format!("GraphQL リクエストの JSON が不正です: {e}") }] }),
            )
        }
    };
    let resp = schema.execute(gql_req).await;
    json_response(StatusCode::OK, &resp)
}

pub fn app(schema: RrdSchema, max_body: usize) -> Route {
    Route::new()
        .at(
            "/",
            get(handler_fn(|_req, _p| async {
                html_response(StatusCode::OK, INDEX_HTML)
            })),
        )
        .at(
            "/healthz",
            get(handler_fn(|_req, _p| async {
                json_response(StatusCode::OK, &serde_json::json!({ "ok": true }))
            })),
        )
        .at(
            "/graphql",
            post(handler_fn(move |req, _p| {
                let schema = schema.clone();
                async move { graphql(schema, req, max_body).await }
            })),
        )
        .with_compression()
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let bind: SocketAddr = std::env::var("RRD_BIND")
        .unwrap_or_else(|_| "127.0.0.1:4701".into())
        .parse()
        .expect("RRD_BIND は ホスト:ポート 形式で指定してください");
    let max_body = std::env::var("RRD_MAX_BODY")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(32 << 20);

    let mut state = AppState::new(
        rrd_compute::default_device(),
        std::env::var("RRD_ARUARU_LLM_URL").unwrap_or_else(|_| "http://127.0.0.1:4600".into()),
        std::env::var("RRD_DATA_DIR")
            .unwrap_or_else(|_| "data".into())
            .into(),
    );
    // 保存先: GitHub の非公開リポジトリ(VPS の DB・ディスクには保存しない)。
    // 接続できなくてもサーバーは起動し、保存・版管理だけ使えない状態にする。
    if let Some(repo) = archive::repo_from_env() {
        match store::Store::connect(&repo).await {
            Ok(s) => state.store = Some(s),
            Err(e) => eprintln!("realdata.pro: 保存先(GitHub)を使えません({e:#})"),
        }
    }
    let state = Arc::new(state);
    println!("realdata.pro: 計算デバイス = {}", state.device.info().name);
    tokio::spawn(schedule(state.clone()));
    tokio::spawn(tuning::ensure_profile(state.clone(), false));
    osm::init(archive::repo_from_env());
    tokio::spawn(load_regions(state.clone()));
    tokio::spawn(load_saved(state.clone()));
    crawl::load_latest(&state);
    let schema = build_schema(state);

    let (addr, handle) = Server::new(TcpListener::bind(bind))
        .run(app(schema, max_body))
        .await?;
    println!("realdata.pro: http://{addr}/ で待受中(GraphQL: POST /graphql)");
    tokio::select! {
        _ = handle => {}
        _ = tokio::signal::ctrl_c() => println!("realdata.pro: 終了します"),
    }
    Ok(())
}

/// 定期実行: 市場データは起動時と 3 時間ごと、外貨定期預金の金利は毎朝 7 時(日本時間)に1回。
async fn schedule(state: Arc<AppState>) {
    schema::refresh_market(&state).await;
    let mut last_market = market::now_unix();
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(300)).await;
        let now = market::now_unix();
        // 検査結果が古くなったら(7日)自動で再検査する
        if tuning::is_stale(&state) {
            tokio::spawn(tuning::ensure_profile(state.clone(), false));
        }
        if now.saturating_sub(last_market) >= 3 * 3600 {
            schema::refresh_market(&state).await;
            last_market = now;
        }
        let (y, m, d, hour) = market::jst(now);
        let last = state
            .deposits
            .read()
            .map(|s| s.collected_at_unix)
            .unwrap_or(0);
        let (ly, lm, ld, _) = market::jst(last);
        if hour >= 7 && (last == 0 || (ly, lm, ld) != (y, m, d)) {
            tokio::spawn(schema::run_deposit_collection(state.clone()));
        }
        if hour >= 7 && crawl::is_due(&state) {
            tokio::spawn(crawl::run_daily(state.clone()));
        }
        // 夜(日本時間の1〜6時)に、地図データの先取得(都道府県×分類、1日に決まった数だけ)
        if osm::refresh_due(u64::from(hour)) {
            let st = state.clone();
            tokio::spawn(async move {
                let regions = st.regions.read().ok().and_then(|g| g.clone());
                let n = std::env::var("RRD_OSM_PAIRS_PER_DAY")
                    .ok()
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(30)
                    .clamp(1, 200);
                if let Some(r) = regions {
                    match osm::refresh(&st.http, &r, n).await {
                        Ok(done) => eprintln!(
                            "realdata.pro: 地図データを {done} 件、先に取得して保存しました"
                        ),
                        Err(e) => eprintln!("realdata.pro: 地図データの先取得に失敗: {e:#}"),
                    }
                }
            });
        }
        // 保存できずに手元に残った収集結果を GitHub へ送り直す(毎日1回)
        if hour >= 7 && crawl::archive_due(&state) {
            tokio::spawn(crawl::archive_old(state.clone()));
        }
    }
}

/// 保存先(GitHub)から、新しいデータセットを読み込む。取得に時間がかかるので、裏で行う。
async fn load_saved(state: Arc<AppState>) {
    let Some(store) = &state.store else { return };
    match store.load_recent(60, 7).await {
        Ok(saved) => {
            let n = saved.len();
            if let Ok(mut map) = state.datasets.write() {
                for (name, csv) in saved {
                    match rrd_core::csv::read_csv_str(&csv) {
                        Ok(df) => {
                            map.entry(name).or_insert(df);
                        }
                        Err(e) => eprintln!("realdata.pro: 保存済みの {name} を読めません: {e}"),
                    }
                }
            }
            println!("realdata.pro: GitHub の保存先から {n} 件のデータセットを読み込みました");
        }
        Err(e) => eprintln!("realdata.pro: 保存済みデータセットの読み込みに失敗: {e:#}"),
    }
}

/// 地名データを読み込む。失敗したら1時間おきに再試行する。
async fn load_regions(state: std::sync::Arc<schema::AppState>) {
    loop {
        match regions::load(&state.http, &state.data_dir).await {
            Ok(r) => {
                println!(
                    "realdata.pro: 地名データを読み込みました(国 {} 件)",
                    r.countries.len()
                );
                if let Ok(mut w) = state.regions.write() {
                    *w = Some(std::sync::Arc::new(r));
                }
                return;
            }
            Err(e) => eprintln!("realdata.pro: 地名データを読み込めません(1時間後に再試行): {e:#}"),
        }
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    }
}
