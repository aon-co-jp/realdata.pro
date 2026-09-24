//! 資金運用・投資の参考情報(公的な一次データ)。
//!
//! - 各国の政策金利: BIS(国際決済銀行)`WS_CBPOL`(約50の国・地域、日次)
//! - 国債利回り: 日本=財務省「国債金利情報」、米国=米国財務省 Daily Treasury Yield Curve、
//!   ユーロ圏=ECB の AAA 国債イールドカーブ
//! - 為替: ECB の参照レート(ユーロ基準)を 1 通貨あたりの円に換算
//!
//! いずれも**日次の公表値**(秒単位のリアルタイムではない)。画面にも出典と基準日を出す。
//! 投資助言ではなく情報提供のみ(画面で明記)。

use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, async_graphql::SimpleObject)]
pub struct PolicyRate {
    /// ISO 3166 の国コード(ユーロ圏は XM)
    pub area: String,
    pub rate: f64,
    pub date: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, async_graphql::SimpleObject)]
pub struct BondYield {
    pub country: String,
    /// 例: "2Y" "10Y" "30Y"
    pub tenor: String,
    pub yield_pct: f64,
    pub date: String,
    pub source: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, async_graphql::SimpleObject)]
pub struct FxRate {
    pub currency: String,
    /// 1 通貨あたりの円
    pub jpy_per_unit: f64,
    pub date: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, async_graphql::SimpleObject)]
pub struct MarketSnapshot {
    pub policy_rates: Vec<PolicyRate>,
    pub bond_yields: Vec<BondYield>,
    pub fx_rates: Vec<FxRate>,
    /// 取得に失敗した情報源(他は表示する)
    pub errors: Vec<String>,
    pub updated_at_unix: u64,
}

const TIMEOUT: Duration = Duration::from_secs(40);

async fn get_text(http: &reqwest::Client, url: &str) -> Result<String> {
    let resp = http
        .get(url)
        .timeout(TIMEOUT)
        .header("user-agent", "realdata.pro/0.1 (+https://realdata.pro)")
        .send()
        .await
        .with_context(|| format!("{url} に接続できません"))?;
    if !resp.status().is_success() {
        bail!("{url}: HTTP {}", resp.status().as_u16());
    }
    Ok(resp.text().await?)
}

/// BIS の各国政策金利(各国の最新の1件)。
pub async fn fetch_policy_rates(http: &reqwest::Client) -> Result<Vec<PolicyRate>> {
    let csv = get_text(
        http,
        "https://stats.bis.org/api/v1/data/WS_CBPOL/D..?lastNObservations=1&format=csv",
    )
    .await?;
    parse_bis_csv(&csv)
}

fn parse_bis_csv(csv: &str) -> Result<Vec<PolicyRate>> {
    let df =
        rrd_core::csv::read_csv_str(csv).map_err(|e| anyhow!("BIS の CSV を読めません: {e}"))?;
    let (area, date, value) = (
        df.column("REF_AREA"),
        df.column("TIME_PERIOD"),
        df.column("OBS_VALUE"),
    );
    let (area, date, value) = (
        area.map_err(|e| anyhow!("{e}"))?,
        date.map_err(|e| anyhow!("{e}"))?,
        value.map_err(|e| anyhow!("{e}"))?,
    );
    let mut out: Vec<PolicyRate> = (0..df.height())
        .filter_map(|i| {
            Some(PolicyRate {
                area: area.get(i).to_string(),
                rate: value.get(i).as_f64()?,
                date: date.get(i).to_string(),
            })
        })
        .filter(|r| !r.area.is_empty())
        .collect();
    out.sort_by(|a, b| a.area.cmp(&b.area));
    out.dedup_by(|a, b| a.area == b.area);
    Ok(out)
}

/// 和暦の基準日(例: "R8.9.18")を ISO 形式にする。令和のみ対応。
fn reiwa_to_iso(s: &str) -> Option<String> {
    let rest = s.trim().strip_prefix('R')?;
    let mut it = rest.split('.');
    let (y, m, d) = (
        it.next()?.parse::<u32>().ok()?,
        it.next()?.parse::<u32>().ok()?,
        it.next()?.parse::<u32>().ok()?,
    );
    (1..=12)
        .contains(&m)
        .then(|| format!("{:04}-{m:02}-{d:02}", 2018 + y))
}

/// 財務省「国債金利情報」(当月分 CSV、Shift_JIS)。最新日の 2・5・10・30 年。
pub async fn fetch_jgb(http: &reqwest::Client) -> Result<Vec<BondYield>> {
    let bytes = http
        .get("https://www.mof.go.jp/jgbs/reference/interest_rate/jgbcm.csv")
        .timeout(TIMEOUT)
        .send()
        .await
        .context("財務省に接続できません")?
        .bytes()
        .await?;
    let (text, _, _) = encoding_rs::SHIFT_JIS.decode(&bytes);
    parse_jgb_csv(&text)
}

fn parse_jgb_csv(text: &str) -> Result<Vec<BondYield>> {
    let lines: Vec<&str> = text.lines().collect();
    let header_idx = lines
        .iter()
        .position(|l| l.starts_with("基準日"))
        .ok_or_else(|| anyhow!("財務省 CSV の見出しがありません"))?;
    let header: Vec<&str> = lines[header_idx].split(',').collect();
    let last = lines[header_idx + 1..]
        .iter()
        .rev()
        .map(|l| l.split(',').collect::<Vec<_>>())
        .find(|f| reiwa_to_iso(f[0]).is_some())
        .ok_or_else(|| anyhow!("財務省 CSV にデータ行がありません"))?;
    let date = reiwa_to_iso(last[0]).unwrap();
    let mut out = Vec::new();
    for (label, tenor) in [
        ("2年", "2Y"),
        ("5年", "5Y"),
        ("10年", "10Y"),
        ("30年", "30Y"),
    ] {
        if let Some(j) = header.iter().position(|h| h.trim() == label) {
            if let Some(v) = last.get(j).and_then(|v| v.trim().parse::<f64>().ok()) {
                out.push(BondYield {
                    country: "JP".into(),
                    tenor: tenor.into(),
                    yield_pct: v,
                    date: date.clone(),
                    source: "財務省".into(),
                });
            }
        }
    }
    if out.is_empty() {
        bail!("財務省 CSV から利回りを読み取れません");
    }
    Ok(out)
}

/// 米国財務省 Daily Treasury Par Yield Curve(当月、無ければ前月)の最新日。
pub async fn fetch_us_treasury(
    http: &reqwest::Client,
    today: (i32, u32),
) -> Result<Vec<BondYield>> {
    let (y, m) = today;
    let prev = if m == 1 { (y - 1, 12) } else { (y, m - 1) };
    for (yy, mm) in [(y, m), prev] {
        let url = format!(
            "https://home.treasury.gov/resource-center/data-chart-center/interest-rates/pages/xml?data=daily_treasury_yield_curve&field_tdr_date_value_month={yy}{mm:02}"
        );
        let xml = get_text(http, &url).await?;
        if let Some(v) = parse_treasury_xml(&xml) {
            return Ok(v);
        }
    }
    bail!("米国財務省のデータに利回りがありません")
}

fn parse_treasury_xml(xml: &str) -> Option<Vec<BondYield>> {
    // 最後の <m:properties> ブロック(最新日)を使う
    let block = xml.rsplit("<m:properties>").next()?;
    let field = |name: &str| -> Option<String> {
        let start = block.find(&format!("<d:{name} "))?;
        let rest = &block[start..];
        let gt = rest.find('>')?;
        let end = rest
            .find('<')
            .filter(|&e| e > gt)
            .or_else(|| rest[gt..].find('<').map(|e| e + gt))?;
        Some(rest[gt + 1..end].trim().to_string())
    };
    let date = field("NEW_DATE")?.chars().take(10).collect::<String>();
    let out: Vec<BondYield> = [
        ("BC_2YEAR", "2Y"),
        ("BC_5YEAR", "5Y"),
        ("BC_10YEAR", "10Y"),
        ("BC_30YEAR", "30Y"),
    ]
    .iter()
    .filter_map(|(f, t)| {
        let v = field(f)?.parse::<f64>().ok()?;
        Some(BondYield {
            country: "US".into(),
            tenor: (*t).into(),
            yield_pct: v,
            date: date.clone(),
            source: "米国財務省".into(),
        })
    })
    .collect();
    (!out.is_empty()).then_some(out)
}

/// ECB のユーロ圏 AAA 国債スポットレート(2・10・30 年)。
pub async fn fetch_ecb_yields(http: &reqwest::Client) -> Result<Vec<BondYield>> {
    let mut out = Vec::new();
    for (key, tenor) in [("SR_2Y", "2Y"), ("SR_10Y", "10Y"), ("SR_30Y", "30Y")] {
        let url = format!("https://data-api.ecb.europa.eu/service/data/YC/B.U2.EUR.4F.G_N_A.SV_C_YM.{key}?lastNObservations=1&format=csvdata");
        let csv = get_text(http, &url).await?;
        let df = rrd_core::csv::read_csv_str(&csv)
            .map_err(|e| anyhow!("ECB の CSV を読めません: {e}"))?;
        let (d, v) = (
            df.column("TIME_PERIOD").map_err(|e| anyhow!("{e}"))?,
            df.column("OBS_VALUE").map_err(|e| anyhow!("{e}"))?,
        );
        if df.height() > 0 {
            if let Some(y) = v.get(df.height() - 1).as_f64() {
                out.push(BondYield {
                    country: "EU".into(),
                    tenor: tenor.into(),
                    yield_pct: (y * 1000.0).round() / 1000.0,
                    date: d.get(df.height() - 1).to_string(),
                    source: "ECB(ユーロ圏 AAA 国債)".into(),
                });
            }
        }
    }
    if out.is_empty() {
        bail!("ECB のデータがありません");
    }
    Ok(out)
}

/// ECB 参照レート(ユーロ基準)を「1 通貨あたりの円」にする。
pub async fn fetch_fx(http: &reqwest::Client) -> Result<Vec<FxRate>> {
    let xml = get_text(
        http,
        "https://www.ecb.europa.eu/stats/eurofxref/eurofxref-daily.xml",
    )
    .await?;
    parse_ecb_fx(&xml)
}

fn parse_ecb_fx(xml: &str) -> Result<Vec<FxRate>> {
    let date = xml
        .split("time='")
        .nth(1)
        .and_then(|s| s.split('\'').next())
        .ok_or_else(|| anyhow!("ECB の為替データに日付がありません"))?
        .to_string();
    let mut per_eur: Vec<(String, f64)> = xml
        .split("currency='")
        .skip(1)
        .filter_map(|s| {
            let cur = s.split('\'').next()?.to_string();
            let rate = s
                .split("rate='")
                .nth(1)?
                .split('\'')
                .next()?
                .parse::<f64>()
                .ok()?;
            Some((cur, rate))
        })
        .collect();
    per_eur.push(("EUR".into(), 1.0));
    let jpy = per_eur
        .iter()
        .find(|(c, _)| c == "JPY")
        .map(|(_, r)| *r)
        .ok_or_else(|| anyhow!("ECB の為替データに円がありません"))?;
    let mut out: Vec<FxRate> = per_eur
        .into_iter()
        .filter(|(c, _)| c != "JPY")
        .map(|(c, r)| FxRate {
            currency: c,
            jpy_per_unit: ((jpy / r) * 10_000.0).round() / 10_000.0,
            date: date.clone(),
        })
        .collect();
    out.sort_by(|a, b| a.currency.cmp(&b.currency));
    Ok(out)
}

/// UNIX 秒 → 日本時間の (年, 月, 日, 時)。暦計算は Howard Hinnant の civil_from_days。
pub fn jst(unix: u64) -> (i32, u32, u32, u32) {
    let secs = unix as i64 + 9 * 3600;
    let days = secs.div_euclid(86_400);
    let hour = (secs.rem_euclid(86_400) / 3600) as u32;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = (yoe + era * 400 + if m <= 2 { 1 } else { 0 }) as i32;
    (y, m, d, hour)
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// すべての情報源をまとめて取得する(失敗した情報源は `errors` に記録し、他は返す)。
pub async fn snapshot(http: &reqwest::Client, today: (i32, u32)) -> MarketSnapshot {
    let (policy, jgb, us, ecb, fx) = tokio::join!(
        fetch_policy_rates(http),
        fetch_jgb(http),
        fetch_us_treasury(http, today),
        fetch_ecb_yields(http),
        fetch_fx(http)
    );
    let mut s = MarketSnapshot::default();
    let mut err = |name: &str, e: anyhow::Error| s.errors.push(format!("{name}: {e:#}"));
    match policy {
        Ok(v) => s.policy_rates = v,
        Err(e) => err("BIS 政策金利", e),
    }
    for (name, r) in [
        ("財務省 国債金利", jgb),
        ("米国財務省", us),
        ("ECB 国債", ecb),
    ] {
        match r {
            Ok(v) => s.bond_yields.extend(v),
            Err(e) => err(name, e),
        }
    }
    match fx {
        Ok(v) => s.fx_rates = v,
        Err(e) => err("ECB 為替", e),
    }
    s.updated_at_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jst_conversion() {
        // 2026-09-24 05:00:00 UTC = 2026-09-24 14:00 JST
        assert_eq!(jst(1_790_226_000), (2026, 9, 24, 14));
        // 2026-12-31 15:30 UTC = 2027-01-01 00:30 JST(年またぎ)
        assert_eq!(jst(1_798_731_000), (2027, 1, 1, 0));
        assert_eq!(jst(0), (1970, 1, 1, 9));
    }

    #[test]
    fn parses_bis_csv_with_quoted_commas() {
        let csv = "FREQ,REF_AREA,COMPILATION,TIME_PERIOD,OBS_VALUE\nD,JP,\"From 1998, call rate\",2026-09-18,0.75\nD,US,x,2026-09-18,4.625\nD,XM,x,2026-09-18,\n";
        let v = parse_bis_csv(csv).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!((v[0].area.as_str(), v[0].rate), ("JP", 0.75));
        assert_eq!(v[1].date, "2026-09-18");
    }

    #[test]
    fn parses_jgb_csv_latest_row() {
        let csv = "国債金利情報 (令和8年9月),,,(単位 : %)\n基準日,1年,2年,5年,10年,30年\nR8.9.17,1.5,1.8,2.3,2.99,4.04\nR8.9.18,1.5,1.849,2.305,2.981,4.044\n,,,,,\n※注記,,,,,\n";
        let v = parse_jgb_csv(csv).unwrap();
        assert_eq!(v.len(), 4);
        let ten = v.iter().find(|b| b.tenor == "10Y").unwrap();
        assert_eq!((ten.yield_pct, ten.date.as_str()), (2.981, "2026-09-18"));
        assert_eq!(reiwa_to_iso("R8.9.18").as_deref(), Some("2026-09-18"));
        assert_eq!(reiwa_to_iso("H30.1.1"), None);
    }

    #[test]
    fn parses_treasury_xml_last_day() {
        let xml = r#"<m:properties><d:NEW_DATE m:type="Edm.DateTime">2026-09-22T00:00:00</d:NEW_DATE><d:BC_10YEAR m:type="Edm.Double">5.0</d:BC_10YEAR></m:properties>
            <m:properties><d:NEW_DATE m:type="Edm.DateTime">2026-09-23T00:00:00</d:NEW_DATE><d:BC_2YEAR m:type="Edm.Double">4.85</d:BC_2YEAR><d:BC_10YEAR m:type="Edm.Double">5.11</d:BC_10YEAR></m:properties>"#;
        let v = parse_treasury_xml(xml).unwrap();
        assert_eq!(v.len(), 2);
        assert!(v.iter().all(|b| b.date == "2026-09-23"));
        assert_eq!(v.iter().find(|b| b.tenor == "10Y").unwrap().yield_pct, 5.11);
    }

    #[test]
    fn converts_ecb_fx_to_jpy() {
        let xml = "<Cube time='2026-09-23'><Cube currency='USD' rate='1.1411'/><Cube currency='JPY' rate='180.20'/></Cube>";
        let v = parse_ecb_fx(xml).unwrap();
        let usd = v.iter().find(|f| f.currency == "USD").unwrap();
        assert!(
            (usd.jpy_per_unit - 157.9178).abs() < 1e-3,
            "{}",
            usd.jpy_per_unit
        );
        assert_eq!(
            v.iter().find(|f| f.currency == "EUR").unwrap().jpy_per_unit,
            180.2
        );
    }
}
