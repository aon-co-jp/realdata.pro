//! 公開の地図データ(OpenStreetMap、ODbL)から、場所ごとの施設を取得する。
//!
//! 検索エンジンの枠を使わず、キー不要・無料の Overpass API に問い合わせる。温泉・神社仏閣・宿泊・酒蔵/ワイナリー/
//! 蒸溜所/ビール醸造所・工場・工務店・不動産会社などが、名前・種類・住所・公式サイトつきで取れる。
//! データは © OpenStreetMap contributors(ODbL)。
//!
//! **時間切れの対策(数の多い分類 — 山梨県の神社仏閣など — は、その場で問い合わせると数十秒〜数分かかる)**:
//! 1. 都道府県 × 分類の結果を、夜のうちに**先に取得して保存**しておく(`refresh`)。時間の制限を長く取り(170秒)、
//!    相手に負担をかけないよう間を空け、1日に決まった数だけ、古いもの・未取得のものから順に進める(全部が90日で一巡)。
//!    保存先は GitHub の非公開リポジトリ(`osm/<場所>/<分類>.json`。VPS には残さない)。
//! 2. 世界リサーチでは、まず保存済みのデータを使う(数秒)。保存がまだ無い場所・分類だけ、その場で問い合わせる(25秒まで)。
//! 3. 国全体は有名な施設(`wikidata` つき)に絞る。

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};
use std::time::Duration;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::places::Target;

/// 混み合っていることが多いので、複数のサーバーを順に試す
const ENDPOINTS: &[&str] = &[
    "https://overpass-api.de/api/interpreter",
    "https://overpass.kumi.systems/api/interpreter",
    "https://maps.mail.ru/osm/tools/overpass/api/interpreter",
];
const USER_AGENT: &str = "realdata.pro (https://realdata.pro/; research tool)";
/// その場で問い合わせるときの待ち時間(世界リサーチ)
const LIVE_TIMEOUT: u64 = 25;
/// 先に取得しておくときの待ち時間(夜の取得)
const BATCH_TIMEOUT: u64 = 170;
/// 保存済みのデータを作り直す間隔(日)
const REFRESH_DAYS: u64 = 90;
/// 先に取得するときの1件あたりの最大の施設数
const BATCH_LIMIT: usize = 60;

/// 分類(知りたい情報の id → 同じ地図データを使うグループ)。宿泊やビールなど、複数の知りたい情報が同じデータを使う。
pub fn group(topic_id: &str) -> Option<&'static str> {
    Some(match topic_id {
        "onsen" => "onsen",
        "shrine_temple" => "shrine_temple",
        "sightseeing" => "sightseeing",
        "scenic" => "scenic",
        "mountain" => "mountain",
        "budget_hotel" | "business_hotel" => "hotel",
        "pension" => "pension",
        "gourmet" => "gourmet",
        "kimono" => "kimono",
        "sake" | "sake_tour" => "sake",
        "wine" | "wine_tour" => "wine",
        "whisky" | "whisky_tour" => "whisky",
        "beer" | "beer_tour" => "beer",
        "manufacturer" | "factory_tour" | "products" => "factory",
        "farm_experience" | "producer_tour" => "farm",
        "timber" => "timber",
        "construction" | "job_construction" => "construction",
        "builder" | "carpenter" => "builder",
        "realestate" => "realestate",
        _ => return None,
    })
}

/// 先に取得しておくグループの一覧
pub const GROUPS: &[&str] = &[
    "onsen",
    "shrine_temple",
    "sightseeing",
    "scenic",
    "mountain",
    "hotel",
    "pension",
    "gourmet",
    "kimono",
    "sake",
    "wine",
    "whisky",
    "beer",
    "factory",
    "farm",
    "timber",
    "construction",
    "builder",
    "realestate",
];

/// グループ → OpenStreetMap のタグ条件(どれかに当てはまるもの)
fn filters_of(group: &str) -> &'static [&'static str] {
    match group {
        "onsen" => &[
            "[natural=hot_spring]",
            "[amenity=public_bath][\"bath:type\"=onsen]",
        ],
        "shrine_temple" => &["[amenity=place_of_worship][religion~\"^(shinto|buddhist)$\"]"],
        "sightseeing" => &["[tourism=attraction]", "[tourism=viewpoint]"],
        "scenic" => &["[natural=lake][name]", "[tourism=viewpoint]"],
        "mountain" => &["[natural=peak][name]", "[tourism=alpine_hut]"],
        "hotel" => &["[tourism=hotel]"],
        "pension" => &["[tourism=guest_house]", "[tourism=chalet]"],
        "gourmet" => &["[amenity=restaurant]"],
        "kimono" => &["[shop=clothes][name~\"着物|きもの|kimono\",i]"],
        "sake" => &["[craft~\"^(sake_brewery|brewery)$\"]"],
        "wine" => &["[craft=winery]", "[tourism=wine_cellar]"],
        "whisky" => &["[craft=distillery]"],
        "beer" => &["[craft=brewery]", "[amenity=biergarten]"],
        "factory" => &["[man_made=works]", "[industrial=factory]"],
        "farm" => &["[tourism=farm]", "[shop=farm]"],
        "timber" => &["[craft=sawmill]", "[industrial=sawmill]"],
        "construction" => &["[office=construction_company]"],
        "builder" => &["[craft=carpenter]", "[craft=builder]"],
        "realestate" => &["[office=estate_agent]"],
        _ => &[],
    }
}

/// 知りたい情報 → OpenStreetMap のタグ条件。地図データに無い情報は None。
pub fn filters(topic_id: &str) -> Option<&'static [&'static str]> {
    group(topic_id).map(filters_of)
}

/// Overpass の正規表現・文字列に入れる名前を安全にする(記号は取り除く)
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

/// 問い合わせ(Overpass QL)を作る。`limit` は返す施設の最大数、`timeout` は Overpass の処理時間の上限(秒)。
pub fn build_query_for(
    t: &Target,
    country_iso: &str,
    group: &str,
    limit: usize,
    timeout: u64,
) -> Result<String> {
    let filters = filters_of(group);
    if filters.is_empty() {
        bail!("この情報は地図データにありません");
    }
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
        "[out:json][timeout:{timeout}];{area}({body});out center tags {limit};"
    ))
}

#[cfg(test)]
pub fn build_query(t: &Target, country_iso: &str, topic_id: &str, limit: u8) -> Result<String> {
    let g = group(topic_id).ok_or_else(|| anyhow::anyhow!("この情報は地図データにありません"))?;
    build_query_for(t, country_iso, g, usize::from(limit), LIVE_TIMEOUT)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
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

async fn fetch_with(http: &reqwest::Client, query: &str, timeout_secs: u64) -> Result<Vec<Place>> {
    let _permit = SLOTS.acquire().await?;
    let mut last = String::new();
    for endpoint in ENDPOINTS {
        let resp = http
            .post(*endpoint)
            .header("User-Agent", USER_AGENT)
            .timeout(Duration::from_secs(timeout_secs + 5))
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

// ───── 保存済みのデータ(GitHub)とメモリの保管 ─────

static REPO: OnceLock<String> = OnceLock::new();
type MemCache = RwLock<HashMap<String, (u64, Vec<Place>)>>;
static MEM: OnceLock<MemCache> = OnceLock::new();

fn mem() -> &'static MemCache {
    MEM.get_or_init(|| RwLock::new(HashMap::new()))
}

/// 保存先(GitHub の非公開リポジトリ)を設定する。未設定なら、その場の問い合わせだけを使う。
pub fn init(repo: Option<String>) {
    if let Some(r) = repo {
        let _ = REPO.set(r);
    }
}

/// ファイル名に使える形にする(日本語などの文字はそのまま、記号は `_` に)
fn file_safe(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .take(60)
        .collect();
    if s.is_empty() {
        "_".into()
    } else {
        s
    }
}

/// 保存先のパス `osm/<場所>/<グループ>.json`
pub fn cache_path(t: &Target, group: &str) -> String {
    format!("osm/{}/{group}.json", file_safe(&t.label))
}

#[derive(Serialize, Deserialize)]
struct Stored {
    fetched_unix: u64,
    places: Vec<Place>,
}

/// 世界リサーチの前に、必要な保存済みデータをまとめて GitHub から読む(1回の取得で足りる)。
pub async fn preload(needs: &[(&Target, &str)]) {
    let Some(repo) = REPO.get() else { return };
    let mut paths: Vec<String> = Vec::new();
    for (t, topic) in needs {
        if let Some(g) = group(topic) {
            let p = cache_path(t, g);
            let known = mem().read().is_ok_and(|m| m.contains_key(&p));
            if !known && !paths.contains(&p) {
                paths.push(p);
            }
        }
    }
    if paths.is_empty() {
        return;
    }
    if let Ok(got) = crate::archive::read_many(repo, "HEAD", &paths).await {
        if let Ok(mut m) = mem().write() {
            for (p, bytes) in got {
                if let Some(s) = bytes.and_then(|b| serde_json::from_slice::<Stored>(&b).ok()) {
                    m.insert(p, (s.fetched_unix, s.places));
                }
            }
        }
    }
}

/// 施設を得る。保存済み(メモリ)があればそれを、無ければその場で問い合わせる(25秒まで)。
pub async fn get(
    http: &reqwest::Client,
    t: &Target,
    topic_id: &str,
    limit: usize,
) -> Result<(Vec<Place>, bool)> {
    let g = group(topic_id).ok_or_else(|| anyhow::anyhow!("この情報は地図データにありません"))?;
    let path = cache_path(t, g);
    if let Some(hit) = mem().read().ok().and_then(|m| m.get(&path).cloned()) {
        return Ok((hit.1.into_iter().take(limit).collect(), true));
    }
    let q = build_query_for(t, &t.iso, g, limit.max(10), LIVE_TIMEOUT)?;
    let places = fetch_with(http, &q, LIVE_TIMEOUT).await?;
    if let Ok(mut m) = mem().write() {
        m.insert(path, (crate::market::now_unix(), places.clone()));
    }
    Ok((places.into_iter().take(limit).collect(), false))
}

// ───── 夜の先取得 ─────

const MANIFEST: &str = "osm/_index.json";

/// 作り直しが必要な(未取得・古い)場所×グループを、未取得 → 古い順に `budget` 件選ぶ。
pub fn stale_pairs(
    manifest: &HashMap<String, u64>,
    now: u64,
    labels: &[String],
    budget: usize,
) -> Vec<(String, &'static str)> {
    let mut v: Vec<(u64, String, &'static str)> = Vec::new();
    for label in labels {
        for &g in GROUPS {
            let path = format!("osm/{}/{g}.json", file_safe(label));
            let at = manifest.get(&path).copied().unwrap_or(0);
            if at == 0 || now.saturating_sub(at) > REFRESH_DAYS * 86_400 {
                v.push((at, label.clone(), g));
            }
        }
    }
    v.sort();
    v.into_iter().take(budget).map(|(_, l, g)| (l, g)).collect()
}

static LAST_REFRESH_DAY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn day_index(unix: u64) -> u64 {
    (unix + 9 * 3600) / 86_400
}

/// 夜(日本時間の1〜6時)に、今日まだ先取得をしていなければ true。
pub fn refresh_due(hour_jst: u64) -> bool {
    REPO.get().is_some()
        && (1..6).contains(&hour_jst)
        && LAST_REFRESH_DAY.load(std::sync::atomic::Ordering::SeqCst)
            != day_index(crate::market::now_unix())
}

/// 都道府県 × グループの地図データを、`budget` 件だけ先に取得して GitHub に保存する。
/// 取得できた件数を返す。1件ずつ間を空け(相手に負担をかけない)、10件ごとにまとめて保存する。
pub async fn refresh(
    http: &reqwest::Client,
    regions: &crate::regions::RegionData,
    budget: usize,
) -> Result<usize> {
    let Some(repo) = REPO.get() else { return Ok(0) };
    let now = crate::market::now_unix();
    LAST_REFRESH_DAY.store(day_index(now), std::sync::atomic::Ordering::SeqCst);
    let prefs = regions.top_level("JP").items;
    let manifest: HashMap<String, u64> =
        crate::archive::read_many(repo, "HEAD", &[MANIFEST.to_string()])
            .await
            .ok()
            .and_then(|v| v.into_iter().next())
            .and_then(|(_, b)| b)
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
    let mut manifest = manifest;
    // 場所の表示名(`places::resolve` の label)で数える
    let mut targets: Vec<(String, Target)> = Vec::new();
    for p in &prefs {
        let place = crate::places::Place {
            country: "JP".into(),
            region: Some(p.code.clone()),
            city: None,
        };
        if let Ok(t) = crate::places::resolve(&place, regions) {
            targets.push((t.label.clone(), t));
        }
    }
    let labels: Vec<String> = targets.iter().map(|(l, _)| l.clone()).collect();
    let todo = stale_pairs(&manifest, now, &labels, budget);
    let mut pending: Vec<(String, Vec<u8>)> = Vec::new();
    let mut done = 0;
    for (label, g) in todo {
        let Some((_, t)) = targets.iter().find(|(l, _)| *l == label) else {
            continue;
        };
        let q = match build_query_for(t, &t.iso, g, BATCH_LIMIT, BATCH_TIMEOUT) {
            Ok(q) => q,
            Err(_) => continue,
        };
        match fetch_with(http, &q, BATCH_TIMEOUT).await {
            Ok(places) => {
                let path = cache_path(t, g);
                let stored = Stored {
                    fetched_unix: crate::market::now_unix(),
                    places: places.clone(),
                };
                if let Ok(bytes) = serde_json::to_vec(&stored) {
                    pending.push((path.clone(), bytes));
                    manifest.insert(path.clone(), stored.fetched_unix);
                    if let Ok(mut m) = mem().write() {
                        m.insert(path, (stored.fetched_unix, places));
                    }
                    done += 1;
                }
            }
            Err(e) => eprintln!("realdata.pro: 地図データの先取得に失敗({label}/{g}): {e:#}"),
        }
        if pending.len() >= 10 {
            flush(repo, &mut pending, &manifest).await;
        }
        // 相手(Overpass)に負担をかけないよう、間を空ける
        tokio::time::sleep(Duration::from_secs(20)).await;
    }
    flush(repo, &mut pending, &manifest).await;
    Ok(done)
}

async fn flush(repo: &str, pending: &mut Vec<(String, Vec<u8>)>, manifest: &HashMap<String, u64>) {
    if pending.is_empty() {
        return;
    }
    if let Ok(m) = serde_json::to_vec(manifest) {
        pending.push((MANIFEST.to_string(), m));
    }
    match crate::archive::push(repo, pending, "osm cache").await {
        Ok(_) => pending.clear(),
        Err(e) => {
            eprintln!("realdata.pro: 地図データの保存に失敗(次回やり直します): {e:#}");
            pending.retain(|(p, _)| p != MANIFEST);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(region: Option<&str>, city: Option<&str>) -> Target {
        Target {
            label: "日本 › 山梨県".into(),
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
    fn batch_queries_use_a_long_timeout_and_live_ones_a_short_one() {
        let t = target(Some("山梨県"), None);
        let q = build_query_for(&t, "JP", "shrine_temple", 60, BATCH_TIMEOUT).unwrap();
        assert!(q.starts_with("[out:json][timeout:170];") && q.ends_with("out center tags 60;"));
        let q = build_query(&t, "JP", "shrine_temple", 10).unwrap();
        assert!(q.starts_with("[out:json][timeout:25];"));
    }

    #[test]
    fn topics_share_groups_and_every_group_has_filters() {
        assert_eq!(group("budget_hotel"), group("business_hotel"));
        assert_eq!(group("sake"), group("sake_tour"));
        assert!(group("stream_fishing").is_none());
        for g in GROUPS {
            assert!(!filters_of(g).is_empty(), "{g}");
        }
        // 知りたい情報のうち、地図データを使うものは、すべてグループの一覧に含まれる
        for tp in crate::places::TOPICS {
            if let Some(g) = group(tp.id) {
                assert!(GROUPS.contains(&g), "{}", tp.id);
            }
        }
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
    fn stale_pairs_prefer_never_fetched_then_oldest_and_skip_fresh() {
        let labels = vec!["日本 › 山梨県".to_string(), "日本 › 長野県".to_string()];
        let now = 200 * 86_400;
        let mut m: HashMap<String, u64> = HashMap::new();
        // 山梨県の onsen は新しい、shrine_temple は古い(100日前)。長野県は全部未取得
        m.insert("osm/日本___山梨県/onsen.json".into(), now - 86_400);
        m.insert(
            "osm/日本___山梨県/shrine_temple.json".into(),
            now - 100 * 86_400,
        );
        let v = stale_pairs(&m, now, &labels, 1000);
        assert!(
            !v.contains(&("日本 › 山梨県".to_string(), "onsen")),
            "新しいものは対象外"
        );
        assert!(
            v.contains(&("日本 › 山梨県".to_string(), "shrine_temple")),
            "90日を超えたら作り直す"
        );
        // 未取得(0)が先、古いものが後
        let pos = |l: &str, g: &str| v.iter().position(|(a, b)| a == l && *b == g).unwrap();
        assert!(
            pos("日本 › 長野県", "onsen") < pos("日本 › 山梨県", "shrine_temple"),
            "未取得のものが、古いものより先"
        );
        assert_eq!(stale_pairs(&m, now, &labels, 3).len(), 3);
    }

    #[test]
    fn cache_paths_are_safe_repository_paths() {
        let p = cache_path(&target(Some("山梨県"), None), "onsen");
        assert_eq!(p, "osm/日本___山梨県/onsen.json");
        assert!(crate::archive::safe_rel(&p));
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
