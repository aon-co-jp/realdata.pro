//! rrd-server: rs-real-data の Web サーバー。
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

mod schema;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};

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

    let state = Arc::new(AppState {
        datasets: RwLock::new(HashMap::new()),
        device: rrd_compute::default_device(),
    });
    println!("rs-real-data: 計算デバイス = {}", state.device.info().name);
    let schema = build_schema(state);

    let (addr, handle) = Server::new(TcpListener::bind(bind))
        .run(app(schema, max_body))
        .await?;
    println!("rs-real-data: http://{addr}/ で待受中(GraphQL: POST /graphql)");
    tokio::select! {
        _ = handle => {}
        _ = tokio::signal::ctrl_c() => println!("rs-real-data: 終了します"),
    }
    Ok(())
}
