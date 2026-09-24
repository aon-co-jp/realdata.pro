//! 外貨定期預金の金利(銀行ごと)の自動収集(毎朝)。
//!
//! 銀行ごとの外貨定期預金金利をまとめて取れる無料の API は無いため、次の方法で集める。
//! 1. 通貨ごとに「外貨定期預金 米ドル 金利」などで検索する(aruaru-llm、日本の地域・言語)。
//! 2. 上位のページを SSRF 対策付きで取得し、本文のテキストを取り出す。
//! 3. AI(aruaru-llm の無料 AI)に「銀行名・期間・年利」を JSON で抜き出させる。
//! 4. **幻覚対策**: 年利の数値(「4.5%」など)と銀行名が、そのページの本文に実際に書かれている
//!    ものだけを採用する。書かれていない値は捨てる。
//!
//! 結果は目安であり、各行に出典 URL と取得日時を付けて表示する(最新の金利は各銀行で確認)。

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::explain::complete;

/// (通貨コード, 日本語名)
pub const CURRENCIES: &[(&str, &str)] = &[
    ("USD", "米ドル"),
    ("EUR", "ユーロ"),
    ("AUD", "豪ドル"),
    ("GBP", "英ポンド"),
    ("NZD", "NZドル"),
    ("CAD", "カナダドル"),
    ("ZAR", "南アフリカランド"),
    ("MXN", "メキシコペソ"),
];

/// 1 通貨あたりに読むページ数(AI の呼び出し回数と検索回数の節約のため)
const PAGES_PER_CURRENCY: usize = 2;
const MAX_PAGE_CHARS: usize = 12_000;

#[derive(Clone, Debug, Default, Serialize, Deserialize, async_graphql::SimpleObject)]
pub struct DepositRate {
    pub currency: String,
    pub bank: String,
    /// 例: "1年" "6か月"
    pub term: String,
    /// 年利(%)
    pub rate: f64,
    /// 条件(預入金額の段階・キャンペーン・新規資金限定など。ページに書かれている範囲)
    #[serde(default)]
    pub condition: String,
    pub url: String,
    pub page_title: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, async_graphql::SimpleObject)]
pub struct DepositSnapshot {
    pub items: Vec<DepositRate>,
    pub errors: Vec<String>,
    /// 収集した日時(UNIX 秒、0 なら未収集)
    pub collected_at_unix: u64,
}

pub fn file_path(data_dir: &Path) -> PathBuf {
    data_dir.join("deposit_rates.json")
}

pub fn load(data_dir: &Path) -> DepositSnapshot {
    std::fs::read_to_string(file_path(data_dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(data_dir: &Path, snap: &DepositSnapshot) -> Result<()> {
    std::fs::create_dir_all(data_dir)
        .with_context(|| format!("{} を作れません", data_dir.display()))?;
    let tmp = data_dir.join("deposit_rates.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(snap)?)?;
    std::fs::rename(&tmp, file_path(data_dir))?;
    Ok(())
}

/// HTML から本文のテキストを取り出す(script / style は除く、空白をまとめる)。
fn page_text(html: &str) -> String {
    let doc = scraper::Html::parse_document(html);
    let body = scraper::Selector::parse("body").expect("固定のセレクタは常に正しい");
    let skip = ["script", "style", "noscript", "svg"];
    let mut out = String::new();
    if let Some(b) = doc.select(&body).next() {
        for node in b.descendants() {
            if let Some(t) = node.value().as_text() {
                let hidden = node.ancestors().any(|a| {
                    a.value()
                        .as_element()
                        .is_some_and(|e| skip.contains(&e.name()))
                });
                if !hidden {
                    out.push_str(t);
                    out.push(' ');
                }
            }
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 年利の数値がページに「%」付きで書かれているか(4.5 → "4.5%" / "4.50 %" / "4.5％" など)。
fn rate_in_text(text: &str, rate: f64) -> bool {
    let mut forms: Vec<String> = Vec::new();
    for decimals in 0..=4 {
        let s = format!("{rate:.decimals$}");
        if s.parse::<f64>()
            .map(|v| (v - rate).abs() < 1e-9)
            .unwrap_or(false)
        {
            forms.push(s);
        }
    }
    let t = text.replace('％', "%");
    forms.iter().any(|f| {
        t.match_indices(f.as_str()).any(|(i, _)| {
            // 数字の途中(例: 14.5 の 4.5)に一致したものは除く
            let before_ok = t[..i]
                .chars()
                .next_back()
                .map_or(true, |c| !c.is_ascii_digit() && c != '.');
            let after = t[i + f.len()..].trim_start();
            let after_ok = !after.starts_with(|c: char| c.is_ascii_digit());
            before_ok && after_ok && after.starts_with('%')
        })
    })
}

/// AI の回答から JSON 配列を取り出す。
fn extract_array(text: &str) -> Option<Vec<Json>> {
    let s = text.find('[')?;
    let e = text.rfind(']')?;
    (e > s)
        .then(|| serde_json::from_str::<Vec<Json>>(&text[s..=e]).ok())
        .flatten()
}

/// 1 ページ分: AI に抜き出させ、本文に書かれているものだけを返す。
async fn extract_from_page(
    http: &reqwest::Client,
    llm: &str,
    cur: (&str, &str),
    url: &str,
    title: &str,
    text: &str,
) -> Result<Vec<DepositRate>> {
    let (code, name) = cur;
    let body: String = text.chars().take(MAX_PAGE_CHARS).collect();
    let prompt = format!(
        "From the web page text below, extract foreign-currency TIME DEPOSIT interest rates for {name} ({code}) only.\n\
         Return ONLY a JSON array like [{{\"bank\": \"bank name as written\", \"term\": \"e.g. 1年 or 6か月\", \"rate\": 4.5, \"condition\": \"short condition as written on the page, e.g. 預入金額100万円以上 / キャンペーン / 新規資金限定; empty if none\"}}].\n\
         If the same bank and term have several rates (amount tiers, campaigns), return one item per rate with its condition.\n\
         rate is the annual interest rate in percent as a number, copied exactly as written on the page. Do not include \
         campaign conditions you cannot read, other currencies, ordinary deposits, or anything not written on the page. \
         If there is none, return [].\n\nPage title: {title}\nPage text:\n{body}"
    );
    let (answer, _) = complete(http, llm, &prompt).await?;
    let arr = extract_array(&answer).ok_or_else(|| anyhow!("AI の回答を読めません"))?;
    let mut out = Vec::new();
    for v in arr {
        let bank = v
            .get("bank")
            .and_then(Json::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let term = v
            .get("term")
            .and_then(Json::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let rate = match v.get("rate") {
            Some(Json::Number(n)) => n.as_f64(),
            Some(Json::String(s)) => s.trim().trim_end_matches(['%', '％']).trim().parse().ok(),
            _ => None,
        };
        let Some(rate) = rate else { continue };
        // 検証: 常識的な範囲・銀行名と年利がページに書かれていること
        if !(0.0..=40.0).contains(&rate)
            || bank.is_empty()
            || bank.chars().count() > 60
            || term.chars().count() > 30
        {
            continue;
        }
        if !body.contains(&bank) || !rate_in_text(&body, rate) {
            continue;
        }
        let condition: String = v
            .get("condition")
            .and_then(Json::as_str)
            .unwrap_or("")
            .trim()
            .chars()
            .take(80)
            .collect();
        out.push(DepositRate {
            currency: code.into(),
            bank,
            term,
            condition,
            rate,
            url: url.into(),
            page_title: title.into(),
        });
    }
    Ok(out)
}

/// 全通貨を収集する(失敗した通貨・ページは errors に記録し、他は返す)。
pub async fn collect(http: &reqwest::Client, llm: &str) -> DepositSnapshot {
    let mut snap = DepositSnapshot::default();
    for &(code, name) in CURRENCIES {
        let query = format!("外貨定期預金 {name} 金利");
        let body = serde_json::json!({ "source": "google", "query": query, "max_results": 5, "gl": "jp", "hl": "ja" });
        let results = match search(http, llm, body).await {
            Ok(r) => r,
            Err(e) => {
                snap.errors.push(format!("{name}: 検索に失敗 ({e:#})"));
                continue;
            }
        };
        let mut pages = 0;
        for r in results {
            if pages >= PAGES_PER_CURRENCY {
                break;
            }
            let url = r
                .get("link")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_string();
            let title = r
                .get("title")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_string();
            if url.is_empty() || url.to_ascii_lowercase().ends_with(".pdf") {
                continue;
            }
            pages += 1;
            let text = match crate::ingest::fetch_page_html(&url).await {
                Ok(html) => page_text(&html),
                Err(e) => {
                    snap.errors
                        .push(format!("{name}: {url} を取得できません ({e:#})"));
                    continue;
                }
            };
            match extract_from_page(http, llm, (code, name), &url, &title, &text).await {
                Ok(v) => snap.items.extend(v),
                Err(e) => snap
                    .errors
                    .push(format!("{name}: {url} の読み取りに失敗 ({e:#})")),
            }
        }
    }
    // 同じ銀行・通貨・期間・年利の重複をまとめ、通貨→年利の高い順に並べる
    snap.items.sort_by(|a, b| {
        (a.currency.as_str(), a.bank.as_str(), a.term.as_str())
            .cmp(&(b.currency.as_str(), b.bank.as_str(), b.term.as_str()))
            .then(b.rate.total_cmp(&a.rate))
    });
    snap.items.dedup_by(|a, b| {
        a.currency == b.currency
            && a.bank == b.bank
            && a.term == b.term
            && a.rate == b.rate
            && a.condition == b.condition
    });
    let order = |c: &str| {
        CURRENCIES
            .iter()
            .position(|x| x.0 == c)
            .unwrap_or(usize::MAX)
    };
    snap.items.sort_by(|a, b| {
        order(&a.currency)
            .cmp(&order(&b.currency))
            .then(b.rate.total_cmp(&a.rate))
    });
    snap.collected_at_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    snap
}

async fn search(http: &reqwest::Client, llm: &str, body: Json) -> Result<Vec<Json>> {
    let resp = http
        .post(format!("{}/v1/search/raw", llm.trim_end_matches('/')))
        .json(&body)
        .timeout(Duration::from_secs(40))
        .send()
        .await
        .context("aruaru-llm に接続できません")?;
    let status = resp.status();
    let j: Json = resp.json().await?;
    if !status.is_success() {
        return Err(anyhow!(
            "{}",
            j.get("error")
                .and_then(Json::as_str)
                .unwrap_or("検索に失敗しました")
        ));
    }
    Ok(j.get("results")
        .and_then(Json::as_array)
        .cloned()
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_must_appear_with_percent() {
        let t = "米ドル定期預金 1年もの 年4.50% キャンペーン 14.5 ポイント 半年 3.8 ％";
        assert!(rate_in_text(t, 4.5));
        assert!(rate_in_text(t, 3.8), "全角％と空白");
        assert!(!rate_in_text(t, 14.5), "% が付いていない");
        assert!(!rate_in_text("年14.5%", 4.5), "数字の途中に一致しない");
        assert!(!rate_in_text(t, 5.0));
    }

    #[test]
    fn page_text_skips_scripts() {
        let html = "<html><head><title>t</title></head><body><script>var a='5.0%'</script><p>A銀行</p><td>年 4.2%</td></body></html>";
        let t = page_text(html);
        assert!(t.contains("A銀行") && t.contains("4.2%"));
        assert!(!t.contains("5.0%"));
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("rrd-deposits-{}", std::process::id()));
        let snap = DepositSnapshot {
            items: vec![DepositRate {
                currency: "USD".into(),
                bank: "A銀行".into(),
                term: "1年".into(),
                rate: 4.2,
                condition: "100万円以上".into(),
                url: "https://e.com".into(),
                page_title: "t".into(),
            }],
            errors: vec![],
            collected_at_unix: 1,
        };
        save(&dir, &snap).unwrap();
        let back = load(&dir);
        assert_eq!(back.items.len(), 1);
        assert_eq!(back.items[0].bank, "A銀行");
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(load(&dir).collected_at_unix, 0, "無ければ空");
    }
}
