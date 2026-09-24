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
//! - `RRD_DB_DSN`  aruaru-db の接続文字列(設定すると保存・版管理が使える。パスワードは表示しない)
//! - `RRD_DATA_DIR`  収集結果の保存先(既定 data)
//! - `RRD_ARUARU_LLM_URL`  aruaru-llm の URL(既定 http://127.0.0.1:4600、検索取り込みと AI 説明に使う)

mod deposits;
mod explain;
mod ingest;
mod languages;
mod market;
mod research;
mod schema;
mod store;

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
    // aruaru-db(版管理)。接続できなくてもサーバーは起動し、版管理だけ使えない状態にする。
    if let Ok(dsn) = std::env::var("RRD_DB_DSN") {
        match store::Store::connect(&dsn).await {
            Ok(s) => {
                match s.load_all().await {
                    Ok(saved) => {
                        let mut map = state
                            .datasets
                            .write()
                            .expect("起動直後のためロックは競合しない");
                        for (name, csv) in saved {
                            match rrd_core::csv::read_csv_str(&csv) {
                                Ok(df) => {
                                    map.insert(name, df);
                                }
                                Err(e) => {
                                    eprintln!("realdata.pro: 保存済みの {name} を読めません: {e}")
                                }
                            }
                        }
                        println!(
                            "realdata.pro: aruaru-db から {} 件のデータセットを読み込みました",
                            map.len()
                        );
                    }
                    Err(e) => {
                        eprintln!("realdata.pro: 保存済みデータセットの読み込みに失敗: {e:#}")
                    }
                }
                state.store = Some(s);
            }
            Err(e) => eprintln!("realdata.pro: 版管理は無効です({e:#})"),
        }
    }
    let state = Arc::new(state);
    println!("realdata.pro: 計算デバイス = {}", state.device.info().name);
    tokio::spawn(schedule(state.clone()));
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
    }
}
