//! 公開の地図データ(OpenStreetMap、ODbL)から、場所ごとの施設を直接取得する。
//!
//! 検索エンジンの無料枠(共有で1日100回)を使わず、キー不要・無料の Overpass API に問い合わせる。
//! 温泉・神社仏閣・宿泊・酒蔵/ワイナリー/蒸溜所/ビール醸造所・工場・工務店・不動産会社などが、
//! 名前・種類・住所・公式サイトつきで取れる。データは © OpenStreetMap contributors(ODbL)。
//! 国全体を調べるときは、件数が多すぎて時間切れにならないよう、有名な施設(`wikidata` つき)に絞る。

use std::time::Duration;

use anyhow::{bail, Result};
use serde_json::Value as Json;

use crate::places::Target;

/// 混み合っていることが多いので、複数のサーバーを順に試す
const ENDPOINTS: &[&str] = &[
    "https://overpass-api.de/api/interpreter",
    "https://overpass.kumi.systems/api/interpreter",
    "https://maps.mail.ru/osm/tools/overpass/api/interpreter",
];
const USER_AGENT: &str = "realdata.pro (https://realdata.pro/; research tool)";

/// 知りたい情報 → OpenStreetMap のタグ条件(どれかに当てはまるもの)
pub fn filters(topic_id: &str) -> Option<&'static [&'static str]> {
    Some(match topic_id {
        "onsen" => &[
            "[natural=hot_spring]",
            "[amenity=public_bath][\"bath:type\"=onsen]",
        ],
        "shrine_temple" => &["[amenity=place_of_worship][religion~\"^(shinto|buddhist)$\"]"],
        "sightseeing" => &["[tourism=attraction]", "[tourism=viewpoint]"],
        "scenic" => &["[natural=lake][name]", "[tourism=viewpoint]"],
        "mountain" => &["[natural=peak][name]", "[tourism=alpine_hut]"],
        "budget_hotel" | "business_hotel" => &["[tourism=hotel]"],
        "pension" => &["[tourism=guest_house]", "[tourism=chalet]"],
        "gourmet" => &["[amenity=restaurant]"],
        "kimono" => &["[shop=clothes][name~\"着物|きもの|kimono\",i]"],
        "sake" | "sake_tour" => &["[craft~\"^(sake_brewery|brewery)$\"]"],
        "wine" | "wine_tour" => &["[craft=winery]", "[tourism=wine_cellar]"],
        "whisky" | "whisky_tour" => &["[craft=distillery]"],
        "beer" | "beer_tour" => &["[craft=brewery]", "[amenity=biergarten]"],
        "manufacturer" | "factory_tour" | "products" => {
            &["[man_made=works]", "[industrial=factory]"]
        }
        "farm_experience" | "producer_tour" => &["[tourism=farm]", "[shop=farm]"],
        "timber" => &["[craft=sawmill]", "[industrial=sawmill]"],
        "construction" | "job_construction" => &["[office=construction_company]"],
        "builder" | "carpenter" => &["[craft=carpenter]", "[craft=builder]"],
        "realestate" => &["[office=estate_agent]"],
        _ => return None,
    })
}

/// Overpass の正規表現に入れる名前を安全にする(記号は取り除く)
fn safe_name(s: &str) -> Option<String> {
    let t: String = s
        .chars()
        .filter(|c| !"\\\"^$.*+?()[]{}|<>;&\n\r".contains(*c))
        .collect();
    let t = t.trim().to_string();
    (!t.is_empty() && t.chars().count() <= 60).then_some(t)
}

fn area_clause(t: &Target, country_iso: &str) -> Result<String> {
    let iso = country_iso.to_ascii_uppercase();
    if !iso.chars().all(|c| c.is_ascii_uppercase()) || iso.len() != 2 {
        bail!("国コードが正しくありません");
    }
    // 名前の完全一致(索引が効いて速い)。日本は日本語名、外国は英語名(GeoNames の表記)で探す。
    let key = if t.hl.starts_with("ja") {
        "name"
    } else {
        "name:en"
    };
    let by_name = |n: &str| -> Result<String> {
        let n = safe_name(n).ok_or_else(|| anyhow::anyhow!("地名が正しくありません"))?;
        Ok(format!(
            "area[boundary=administrative][\"{key}\"=\"{n}\"]->.a;"
        ))
    };
    if let Some(city) = &t.city {
        return by_name(city);
    }
    if let Some(region) = &t.region {
        return by_name(region);
    }
    Ok(format!(
        "area[boundary=administrative][\"ISO3166-1\"=\"{iso}\"]->.a;"
    ))
}

pub fn build_query(t: &Target, country_iso: &str, topic_id: &str, limit: u8) -> Result<String> {
    let filters =
        filters(topic_id).ok_or_else(|| anyhow::anyhow!("この情報は地図データにありません"))?;
    let area = area_clause(t, country_iso)?;
    // 国全体は、有名な施設(wikidata つき)だけにする。それ以外は名前のあるものすべて。
    let extra = if t.whole_country {
        "[wikidata]"
    } else {
        "[name]"
    };
    let body: String = filters
        .iter()
        .map(|f| format!("nwr(area.a){f}{extra};"))
        .collect();
    Ok(format!(
        "[out:json][timeout:25];{area}({body});out center tags {limit};"
    ))
}

pub struct Place {
    pub name: String,
    pub kind: String,
    pub address: String,
    pub url: String,
}

fn tag<'a>(tags: &'a Json, k: &str) -> Option<&'a str> {
    tags.get(k).and_then(Json::as_str).filter(|s| !s.is_empty())
}

pub fn parse(json: &Json) -> Vec<Place> {
    let Some(els) = json.get("elements").and_then(Json::as_array) else {
        return Vec::new();
    };
    els.iter()
        .filter_map(|e| {
            let tags = e.get("tags")?;
            let name = tag(tags, "name:ja")
                .or_else(|| tag(tags, "name"))
                .or_else(|| tag(tags, "name:en"))?
                .to_string();
            let kind = [
                "tourism",
                "amenity",
                "craft",
                "natural",
                "man_made",
                "industrial",
                "shop",
                "office",
            ]
            .iter()
            .find_map(|k| tag(tags, k).map(|v| format!("{k}={v}")))
            .unwrap_or_default();
            let address = [
                "addr:province",
                "addr:city",
                "addr:suburb",
                "addr:street",
                "addr:housenumber",
            ]
            .iter()
            .filter_map(|k| tag(tags, k))
            .collect::<Vec<_>>()
            .join(" ");
            let ty = e.get("type").and_then(Json::as_str).unwrap_or("node");
            let id = e.get("id").and_then(Json::as_i64).unwrap_or(0);
            let url = tag(tags, "website")
                .or_else(|| tag(tags, "contact:website"))
                .filter(|u| u.starts_with("http://") || u.starts_with("https://"))
                .map_or_else(
                    || format!("https://www.openstreetmap.org/{ty}/{id}"),
                    str::to_string,
                );
            Some(Place {
                name,
                kind,
                address,
                url,
            })
        })
        .collect()
}

/// Overpass API は同時利用に制限があるので、2件までにする。
static SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

pub async fn fetch(http: &reqwest::Client, query: &str) -> Result<Vec<Place>> {
    let _permit = SLOTS.acquire().await?;
    let mut last = String::new();
    for endpoint in ENDPOINTS {
        let resp = http
            .post(*endpoint)
            .header("User-Agent", USER_AGENT)
            .timeout(Duration::from_secs(40))
            .form(&[("data", query)])
            .send()
            .await;
        match resp {
            Ok(r) if r.status().is_success() => {
                let json: Json = r.json().await?;
                return Ok(parse(&json));
            }
            Ok(r) => last = format!("HTTP {}", r.status().as_u16()),
            Err(e) => last = e.to_string(),
        }
    }
    bail!("地図データを取得できませんでした({last})")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(region: Option<&str>, city: Option<&str>) -> Target {
        Target {
            label: "x".into(),
            country_en: "Japan".into(),
            iso: "JP".into(),
            country_ja: "日本".into(),
            gl: "jp".into(),
            hl: "ja".into(),
            lang: "ja",
            place_text: "x".into(),
            whole_country: region.is_none() && city.is_none(),
            region: region.map(String::from),
            city: city.map(String::from),
        }
    }

    #[test]
    fn query_uses_the_narrowest_place_and_notable_only_for_countries() {
        let q = build_query(&target(Some("山梨県"), None), "JP", "onsen", 10).unwrap();
        assert!(
            q.contains("\"name\"=\"山梨県\"")
                && q.contains("[natural=hot_spring][name]")
                && q.ends_with("out center tags 10;")
        );
        let q = build_query(&target(Some("東京都"), Some("渋谷区")), "JP", "onsen", 10).unwrap();
        assert!(q.contains("=\"渋谷区\"") && !q.contains("=\"東京都\""));
        let q = build_query(&target(None, None), "jp", "shrine_temple", 5).unwrap();
        assert!(q.contains("\"ISO3166-1\"=\"JP\"") && q.contains("[wikidata]"));
        assert!(build_query(&target(None, None), "JP", "kimono_nope", 5).is_err());
    }

    #[test]
    fn names_with_regex_or_query_syntax_are_neutralised() {
        assert_eq!(safe_name("a.b\"c;d").as_deref(), Some("abcd"));
        assert!(safe_name("...").is_none());
        let q = build_query(&target(Some("X\"];out;"), None), "JP", "onsen", 5).unwrap();
        assert!(!q.contains("X\"];out;"));
        assert!(build_query(&target(None, None), "J\"P", "onsen", 5).is_err());
    }

    #[test]
    fn parses_elements_with_website_or_osm_link() {
        let j: Json = serde_json::from_str(
            r#"{"elements":[
              {"type":"node","id":1,"tags":{"name":"A温泉","natural":"hot_spring","website":"https://a.example/"}},
              {"type":"way","id":2,"tags":{"name:ja":"B神社","amenity":"place_of_worship","addr:city":"甲府市","website":"javascript:x"}},
              {"type":"node","id":3,"tags":{"amenity":"restaurant"}}]}"#,
        )
        .unwrap();
        let v = parse(&j);
        assert_eq!(v.len(), 2, "名前の無いものは除く");
        assert_eq!(v[0].url, "https://a.example/");
        assert_eq!(
            v[1].url, "https://www.openstreetmap.org/way/2",
            "危険な URL は使わない"
        );
        assert_eq!(v[1].address, "甲府市");
        assert_eq!(v[0].kind, "natural=hot_spring");
    }
}
