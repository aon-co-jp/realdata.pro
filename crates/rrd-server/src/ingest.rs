//! CSV 以外の取り込み元: 検索ワード(Google / YouTube / GitHub)と調査対象 URL。
//!
//! - 検索は aruaru-llm の `POST /v1/search/raw` に任せる(検索 API の実装・キー管理を重複させない)。
//! - URL は本サーバーが取得する。SSRF 対策として、http/https のみを許可し、宛先 IP を検証する。
//!   私設・ループバック・リンクローカル等の IP は拒否し、検証した IP へ固定して接続する
//!   (DNS の再解決による回避を防ぐ)。リダイレクトは1段ごとに同じ検証をかける。
//!   応答サイズと時間にも上限を設けている。

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use rrd_core::DataFrame;
use scraper::{Html, Selector};
use serde_json::Value as Json;

/// URL 取得の上限。
const MAX_FETCH_BYTES: usize = 16 << 20;
const MAX_REDIRECTS: usize = 5;
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);

/// 取り込み結果(データと、どの方法で表にしたか)。
pub struct Imported {
    pub df: DataFrame,
    pub method: String,
}

// ───────────────────────── 検索(aruaru-llm 経由) ─────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchSource {
    Google,
    Youtube,
    Github,
}

impl SearchSource {
    fn as_str(self) -> &'static str {
        match self {
            SearchSource::Google => "google",
            SearchSource::Youtube => "youtube",
            SearchSource::Github => "github",
        }
    }
}

pub async fn import_search(
    http: &reqwest::Client,
    llm_base: &str,
    source: SearchSource,
    query: &str,
    max_results: u8,
) -> Result<Imported> {
    let resp = http
        .post(format!("{}/v1/search/raw", llm_base.trim_end_matches('/')))
        .json(&serde_json::json!({ "source": source.as_str(), "query": query, "max_results": max_results }))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .with_context(|| format!("aruaru-llm({llm_base})に接続できません"))?;
    let status = resp.status();
    let body: Json = resp.json().await.context("aruaru-llm の応答を読めません")?;
    if !status.is_success() {
        let msg = body
            .get("error")
            .and_then(Json::as_str)
            .unwrap_or("不明なエラー");
        bail!("{}検索に失敗しました: {msg}", source.as_str());
    }
    let results = body
        .get("results")
        .and_then(Json::as_array)
        .ok_or_else(|| anyhow!("aruaru-llm の応答に results がありません"))?;
    let df = objects_to_frame(results, Some("rank"))?;
    Ok(Imported {
        df,
        method: format!("{}検索({}件)", source.as_str(), results.len()),
    })
}

/// JSON オブジェクトの配列を表にする(列はキーの出現順の和集合、値は文字列化)。
fn objects_to_frame(items: &[Json], rank_col: Option<&str>) -> Result<DataFrame> {
    let mut keys: Vec<String> = Vec::new();
    for it in items {
        let obj = it
            .as_object()
            .ok_or_else(|| anyhow!("配列の要素がオブジェクトではありません"))?;
        for k in obj.keys() {
            if !keys.contains(k) {
                keys.push(k.clone());
            }
        }
    }
    // 検索結果の見やすさのため、よく使う列を先頭に寄せる。
    for pref in [
        "url",
        "link",
        "stars",
        "channel_title",
        "snippet",
        "description",
        "full_name",
        "title",
    ] {
        if let Some(p) = keys.iter().position(|k| k == pref) {
            let k = keys.remove(p);
            keys.insert(0, k);
        }
    }
    let mut header = keys.clone();
    if let Some(r) = rank_col {
        header.insert(0, r.to_string());
    }
    let rows = items
        .iter()
        .enumerate()
        .map(|(i, it)| {
            let mut row: Vec<Option<String>> =
                keys.iter().map(|k| it.get(k).and_then(json_cell)).collect();
            if rank_col.is_some() {
                row.insert(0, Some((i + 1).to_string()));
            }
            row
        })
        .collect();
    rrd_core::csv::from_records(header, rows).map_err(|e| anyhow!("{e}"))
}

fn json_cell(v: &Json) -> Option<String> {
    match v {
        Json::Null => None,
        Json::String(s) => Some(s.clone()),
        Json::Number(n) => Some(n.to_string()),
        Json::Bool(b) => Some(b.to_string()),
        other => Some(other.to_string()),
    }
}

// ───────────────────────── 調査対象 URL ─────────────────────────

/// 取得を許可しない IP(私設・ループバック・リンクローカル・CGNAT・マルチキャスト等)。
fn is_forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                || o[0] == 0
                || (o[0] == 100 && (64..128).contains(&o[1])) // CGNAT 100.64/10
                || (o[0] == 192 && o[1] == 0 && o[2] == 0) // 192.0.0.0/24
                || (o[0] == 198 && (18..20).contains(&o[1])) // ベンチマーク 198.18/15
                || o[0] >= 240
        }
        IpAddr::V6(v6) => {
            let s = v6.segments();
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_forbidden_ip(IpAddr::V4(v4));
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00 // ユニークローカル fc00::/7
                || (s[0] & 0xffc0) == 0xfe80 // リンクローカル fe80::/10
                || (s[0] == 0x2001 && s[1] == 0x0db8) // ドキュメント用
                || (s[0] == 0x64 && s[1] == 0xff9b) // NAT64(内部 IPv4 への迂回を防ぐ)
        }
    }
}

/// URL を検証し、接続先として許可された IP を1つ返す。
async fn resolve_public(url: &reqwest::Url) -> Result<SocketAddr> {
    match url.scheme() {
        "http" | "https" => {}
        s => bail!("{s}: は取得できません(http / https のみ)"),
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("URL に認証情報を含めないでください");
    }
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("URL にホスト名がありません"))?;
    let port = url.port_or_known_default().unwrap_or(443);
    let host_for_lookup = host.trim_start_matches('[').trim_end_matches(']');
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host_for_lookup, port))
        .await
        .with_context(|| format!("{host} の名前解決に失敗しました"))?
        .collect();
    if addrs.is_empty() {
        bail!("{host} の IP アドレスが見つかりません");
    }
    // 1つでも内部向けの IP を含む名前は拒否する(混在させた回避を防ぐ)。
    if let Some(bad) = addrs.iter().find(|a| is_forbidden_ip(a.ip())) {
        bail!(
            "{host}({})は内部ネットワークのアドレスのため取得できません",
            bad.ip()
        );
    }
    Ok(addrs[0])
}

/// 検証済み IP に固定して取得する(リダイレクトは手動で追い、毎回検証する)。
async fn fetch_public(url: &str) -> Result<(reqwest::Url, String, bytes::Bytes)> {
    let mut url = reqwest::Url::parse(url.trim()).context("URL の形式が正しくありません")?;
    for _ in 0..=MAX_REDIRECTS {
        let addr = resolve_public(&url).await?;
        let host = url
            .host_str()
            .unwrap_or_default()
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_string();
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(FETCH_TIMEOUT)
            .user_agent("realdata.pro/0.1 (+https://realdata.pro)")
            .resolve(&host, addr)
            .build()?;
        let mut resp = client
            .get(url.clone())
            .send()
            .await
            .with_context(|| format!("{url} を取得できません"))?;
        if resp.status().is_redirection() {
            let loc = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| anyhow!("リダイレクト先がありません"))?;
            url = url.join(loc).context("リダイレクト先の URL が不正です")?;
            continue;
        }
        if !resp.status().is_success() {
            bail!(
                "{url} の取得に失敗しました(HTTP {})",
                resp.status().as_u16()
            );
        }
        if resp
            .content_length()
            .is_some_and(|n| n as usize > MAX_FETCH_BYTES)
        {
            bail!("ページが大きすぎます(上限 {} MiB)", MAX_FETCH_BYTES >> 20);
        }
        let ctype = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        let mut buf = bytes::BytesMut::new();
        while let Some(chunk) = resp.chunk().await? {
            if buf.len() + chunk.len() > MAX_FETCH_BYTES {
                bail!("ページが大きすぎます(上限 {} MiB)", MAX_FETCH_BYTES >> 20);
            }
            buf.extend_from_slice(&chunk);
        }
        return Ok((url, ctype, buf.freeze()));
    }
    bail!("リダイレクトが多すぎます(上限 {MAX_REDIRECTS} 回)")
}

/// 公開ページの HTML を取得する(SSRF 対策・サイズ/時間の上限は `fetch_public` と同じ)。
pub async fn fetch_page_html(url: &str) -> Result<String> {
    let (_, ctype, body) = fetch_public(url).await?;
    if !ctype.is_empty() && !ctype.contains("html") && !ctype.contains("text/plain") {
        bail!("HTML ではありません({ctype})");
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// 調査対象 URL を取り込む。CSV → JSON(オブジェクト配列)→ HTML の表 → HTML のリンク一覧の順に試す。
pub async fn import_url(url: &str) -> Result<Imported> {
    let (final_url, ctype, body) = fetch_public(url).await?;
    let text = String::from_utf8_lossy(&body).into_owned();
    let path = final_url.path().to_ascii_lowercase();

    if ctype.contains("csv") || path.ends_with(".csv") {
        let df =
            rrd_core::csv::read_csv_str(&text).map_err(|e| anyhow!("CSV として読めません: {e}"))?;
        return Ok(Imported {
            df,
            method: "CSV".into(),
        });
    }
    if ctype.contains("json") || path.ends_with(".json") {
        let v: Json = serde_json::from_str(&text).context("JSON として読めません")?;
        let arr = match &v {
            Json::Array(a) => a.as_slice(),
            Json::Object(o) => o
                .values()
                .find_map(|x| x.as_array())
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            _ => &[],
        };
        if arr.is_empty() || !arr.iter().all(Json::is_object) {
            bail!("JSON に表にできる配列(オブジェクトの配列)がありません");
        }
        return Ok(Imported {
            df: objects_to_frame(arr, None)?,
            method: format!("JSON({}件)", arr.len()),
        });
    }
    html_to_frame(&text, &final_url)
}

/// 表のヘッダと行。
type TableRows = (Vec<String>, Vec<Vec<Option<String>>>);

/// HTML から最も大きい表を取り出す。表が無ければリンク一覧を表にする。
fn html_to_frame(html: &str, base: &reqwest::Url) -> Result<Imported> {
    let doc = Html::parse_document(html);
    let sel = |s: &str| Selector::parse(s).expect("固定のセレクタは常に正しい");
    let (table_sel, tr_sel, cell_sel) = (sel("table"), sel("tr"), sel("th, td"));
    let clean = |s: String| s.split_whitespace().collect::<Vec<_>>().join(" ");

    let mut best: Option<TableRows> = None;
    for table in doc.select(&table_sel) {
        let rows: Vec<Vec<String>> = table
            .select(&tr_sel)
            .map(|tr| {
                tr.select(&cell_sel)
                    .map(|c| clean(c.text().collect()))
                    .collect::<Vec<_>>()
            })
            .filter(|r: &Vec<String>| !r.is_empty())
            .collect();
        if rows.len() < 2 {
            continue;
        }
        let width = rows.iter().map(Vec::len).max().unwrap_or(0);
        let header = rows[0]
            .clone()
            .into_iter()
            .chain(std::iter::repeat(String::new()))
            .take(width)
            .collect();
        let body: Vec<Vec<Option<String>>> = rows[1..]
            .iter()
            .map(|r| {
                (0..width)
                    .map(|j| r.get(j).cloned().filter(|s| !s.is_empty()))
                    .collect()
            })
            .collect();
        if best.as_ref().map_or(true, |(_, b)| body.len() > b.len()) {
            best = Some((header, body));
        }
    }
    if let Some((header, body)) = best {
        let n = body.len();
        let df = rrd_core::csv::from_records(header, body).map_err(|e| anyhow!("{e}"))?;
        return Ok(Imported {
            df,
            method: format!("HTMLの表({n}行)"),
        });
    }

    // 表が無いページは、リンク一覧(テキスト・URL・同一サイトか)を分析対象にする。
    let a_sel = sel("a[href]");
    let mut rows = Vec::new();
    for a in doc.select(&a_sel) {
        let href = a.value().attr("href").unwrap_or_default();
        let Ok(abs) = base.join(href) else { continue };
        if !matches!(abs.scheme(), "http" | "https") {
            continue;
        }
        let internal = abs.host_str() == base.host_str();
        rows.push(vec![
            Some(clean(a.text().collect())).filter(|s| !s.is_empty()),
            Some(abs.to_string()),
            Some(abs.host_str().unwrap_or_default().to_string()),
            Some(internal.to_string()),
        ]);
    }
    if rows.is_empty() {
        bail!("このページには表もリンクも見つかりませんでした");
    }
    let n = rows.len();
    let df = rrd_core::csv::from_records(
        vec![
            "text".into(),
            "url".into(),
            "host".into(),
            "internal".into(),
        ],
        rows,
    )
    .map_err(|e| anyhow!("{e}"))?;
    Ok(Imported {
        df,
        method: format!("HTMLのリンク一覧({n}件、表が無いページのため)"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbids_internal_addresses() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fe80::1",
            "fd00::1",
            "::ffff:127.0.0.1",
            "64:ff9b::a00:1",
        ] {
            assert!(is_forbidden_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "160.251.237.162", "2606:4700:4700::1111"] {
            assert!(!is_forbidden_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    #[tokio::test]
    async fn rejects_bad_urls() {
        for u in [
            "file:///etc/passwd",
            "ftp://example.com/x",
            "http://127.0.0.1:4701/",
            "http://localhost/",
            "http://user:pw@example.com/",
        ] {
            assert!(fetch_public(u).await.is_err(), "{u}");
        }
    }

    #[test]
    fn html_largest_table_and_links() {
        let base = reqwest::Url::parse("https://example.com/p").unwrap();
        let html = r#"<table><tr><th>a</th></tr><tr><td>1</td></tr></table>
            <table><tr><th>city</th><th>sales</th></tr><tr><td>Tokyo</td><td>100</td></tr>
            <tr><td>Osaka</td><td>80</td></tr></table>"#;
        let im = html_to_frame(html, &base).unwrap();
        assert_eq!(im.df.height(), 2);
        assert_eq!(
            im.df.column("sales").unwrap().dtype(),
            rrd_core::DataType::Int
        );
        let im = html_to_frame(
            r#"<a href="/x">X</a><a href="https://other.org/">O</a><a href="mailto:a@b">m</a>"#,
            &base,
        )
        .unwrap();
        assert_eq!(im.df.height(), 2);
        assert_eq!(im.df.column("internal").unwrap().get(0).to_string(), "true");
    }

    #[test]
    fn search_results_become_table_with_rank() {
        let items: Vec<Json> = serde_json::from_str(
            r#"[{"full_name":"a/b","stars":10,"url":"https://github.com/a/b","description":"x"},
                {"full_name":"c/d","stars":5,"url":"https://github.com/c/d","description":null}]"#,
        )
        .unwrap();
        let df = objects_to_frame(&items, Some("rank")).unwrap();
        assert_eq!(df.columns()[0].name, "rank");
        assert_eq!(df.column("stars").unwrap().dtype(), rrd_core::DataType::Int);
        assert_eq!(df.column("description").unwrap().null_count(), 1);
    }
}
