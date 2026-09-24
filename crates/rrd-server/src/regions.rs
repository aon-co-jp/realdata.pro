//! 世界の地名(国 → 都道府県・州 → 市区町村・都市)。
//!
//! - 世界: GeoNames(<https://www.geonames.org/>、CC BY 4.0)の `countryInfo.txt`・`admin1CodesASCII.txt`・
//!   `cities15000.zip`(人口1.5万人以上の都市)。州・地方は約4,000、都市は約2.5万。
//! - 日本: geolonia japanese-addresses(<https://github.com/geolonia/japanese-addresses>、CC BY 4.0。
//!   国土地理院・総務省のデータ由来)の `ja.json`。都道府県47と、全国の市区町村を日本語名で持つ。
//!   GeoNames の日本は人口1.5万人未満の町村が抜けるため、日本だけはこちらを使う。
//!
//! 取得したファイルは `RRD_DATA_DIR/geo/` に保存し、30日ごとに更新する。取得できなければ保存済みを使う。
//! 地名の**名前は原典どおり**(外国は英語表記のASCII、日本は日本語)。

use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;

const REFRESH_SECS: u64 = 30 * 24 * 3600;
const GEONAMES: &str = "https://download.geonames.org/export/dump";
const JA_JSON: &str = "https://geolonia.github.io/japanese-addresses/api/ja.json";

#[derive(Clone, Debug, Serialize)]
pub struct CountryRec {
    pub code: String,
    pub name: String,
    /// GeoNames の言語(例: "zh-CN,yue,wuu")
    pub languages: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct RegionItem {
    pub code: String,
    pub name: String,
}

pub struct RegionData {
    pub countries: Vec<CountryRec>,
    /// 国コード → 州・地方(名前順)
    admin1: HashMap<String, Vec<RegionItem>>,
    /// "国コード.admin1コード" → 都市(人口の多い順)
    cities: HashMap<String, Vec<RegionItem>>,
    /// 州・地方を持たない国の都市(人口の多い順)
    cities_no_admin: HashMap<String, Vec<RegionItem>>,
    /// 日本: 都道府県(JIS 順)→ 市区町村
    japan: Vec<(String, Vec<String>)>,
}

/// 日本の都道府県(JIS X 0401 の順)。ja.json は順序を保証しないため、表示順はこれで決める。
const PREFECTURES: [&str; 47] = [
    "北海道",
    "青森県",
    "岩手県",
    "宮城県",
    "秋田県",
    "山形県",
    "福島県",
    "茨城県",
    "栃木県",
    "群馬県",
    "埼玉県",
    "千葉県",
    "東京都",
    "神奈川県",
    "新潟県",
    "富山県",
    "石川県",
    "福井県",
    "山梨県",
    "長野県",
    "岐阜県",
    "静岡県",
    "愛知県",
    "三重県",
    "滋賀県",
    "京都府",
    "大阪府",
    "兵庫県",
    "奈良県",
    "和歌山県",
    "鳥取県",
    "島根県",
    "岡山県",
    "広島県",
    "山口県",
    "徳島県",
    "香川県",
    "愛媛県",
    "高知県",
    "福岡県",
    "佐賀県",
    "長崎県",
    "熊本県",
    "大分県",
    "宮崎県",
    "鹿児島県",
    "沖縄県",
];

/// 1つの階層の一覧と、その呼び方。
#[derive(Clone, Debug, Serialize)]
pub struct Children {
    /// "region"(都道府県・州)または "city"(市区町村・都市)
    pub level: String,
    /// 画面に出す呼び方(例: "都道府県" "市区町村")
    pub label: String,
    pub items: Vec<RegionItem>,
}

fn is_fresh(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age.as_secs() < REFRESH_SECS)
}

/// URL を取得して保存する。新しい保存済みファイルがあれば取得しない。失敗しても、保存済みがあればそれを使う。
async fn ensure_file(http: &reqwest::Client, url: &str, path: &Path) -> Result<Vec<u8>> {
    if is_fresh(path) {
        return std::fs::read(path).with_context(|| format!("{} を読めません", path.display()));
    }
    let got = async {
        let resp = http
            .get(url)
            .timeout(Duration::from_secs(120))
            .send()
            .await
            .with_context(|| format!("{url} に接続できません"))?;
        if !resp.status().is_success() {
            bail!("{url}: HTTP {}", resp.status().as_u16());
        }
        Ok(resp.bytes().await?.to_vec())
    }
    .await;
    match got {
        Ok(bytes) => {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let tmp = path.with_extension("tmp");
            if std::fs::write(&tmp, &bytes).is_ok() {
                let _ = std::fs::rename(&tmp, path);
            }
            Ok(bytes)
        }
        Err(e) => match std::fs::read(path) {
            Ok(old) => {
                eprintln!(
                    "realdata.pro: 地名データの更新に失敗したため、保存済みを使います({e:#})"
                );
                Ok(old)
            }
            Err(_) => Err(e),
        },
    }
}

pub fn geo_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("geo")
}

pub async fn load(http: &reqwest::Client, data_dir: &Path) -> Result<RegionData> {
    let dir = geo_dir(data_dir);
    let country_txt = ensure_file(
        http,
        &format!("{GEONAMES}/countryInfo.txt"),
        &dir.join("countryInfo.txt"),
    )
    .await?;
    let admin1_txt = ensure_file(
        http,
        &format!("{GEONAMES}/admin1CodesASCII.txt"),
        &dir.join("admin1CodesASCII.txt"),
    )
    .await?;
    let cities_zip = ensure_file(
        http,
        &format!("{GEONAMES}/cities15000.zip"),
        &dir.join("cities15000.zip"),
    )
    .await?;
    let ja_json = ensure_file(http, JA_JSON, &dir.join("ja.json")).await?;
    // 解析は CPU を使うので、非同期の実行スレッドを塞がない
    tokio::task::spawn_blocking(move || parse(&country_txt, &admin1_txt, &cities_zip, &ja_json))
        .await
        .map_err(|e| anyhow!("地名データの解析が異常終了しました: {e}"))?
}

pub fn parse(
    country_txt: &[u8],
    admin1_txt: &[u8],
    cities_zip: &[u8],
    ja_json: &[u8],
) -> Result<RegionData> {
    let mut countries = Vec::new();
    for line in String::from_utf8_lossy(country_txt)
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() > 15 && !f[0].is_empty() {
            countries.push(CountryRec {
                code: f[0].to_string(),
                name: f[4].to_string(),
                languages: f[15].to_string(),
            });
        }
    }
    if countries.len() < 200 {
        bail!("国の一覧を読めません({}件)", countries.len());
    }
    countries.sort_by(|a, b| a.name.cmp(&b.name));

    let mut admin1: HashMap<String, Vec<RegionItem>> = HashMap::new();
    let mut admin1_names: HashMap<String, String> = HashMap::new();
    for line in String::from_utf8_lossy(admin1_txt).lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 3 {
            continue;
        }
        let Some((cc, code)) = f[0].split_once('.') else {
            continue;
        };
        // 名前は ASCII 版(f[2])、無ければ通常の名前
        let name = if f[2].is_empty() { f[1] } else { f[2] }.to_string();
        admin1_names.insert(f[0].to_string(), name.clone());
        admin1.entry(cc.to_string()).or_default().push(RegionItem {
            code: code.to_string(),
            name,
        });
    }
    for v in admin1.values_mut() {
        v.sort_by(|a, b| a.name.cmp(&b.name));
    }

    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(cities_zip))
        .context("都市データ(zip)を開けません")?;
    let mut txt = String::new();
    zip.by_name("cities15000.txt")
        .context("zip に cities15000.txt がありません")?
        .read_to_string(&mut txt)?;
    let mut raw: HashMap<String, Vec<(i64, RegionItem)>> = HashMap::new();
    let mut raw_no_admin: HashMap<String, Vec<(i64, RegionItem)>> = HashMap::new();
    for line in txt.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 15 {
            continue;
        }
        let (cc, a1, pop) = (f[8], f[10], f[14].parse::<i64>().unwrap_or(0));
        let name = if f[2].is_empty() { f[1] } else { f[2] }.to_string();
        let item = RegionItem {
            code: f[0].to_string(),
            name,
        };
        if admin1_names.contains_key(&format!("{cc}.{a1}")) {
            raw.entry(format!("{cc}.{a1}"))
                .or_default()
                .push((pop, item));
        } else {
            raw_no_admin
                .entry(cc.to_string())
                .or_default()
                .push((pop, item));
        }
    }
    let finish = |m: HashMap<String, Vec<(i64, RegionItem)>>| -> HashMap<String, Vec<RegionItem>> {
        m.into_iter()
            .map(|(k, mut v)| {
                v.sort_by_key(|x| std::cmp::Reverse(x.0));
                (k, v.into_iter().map(|x| x.1).collect())
            })
            .collect()
    };

    let ja: BTreeMap<String, Vec<String>> =
        serde_json::from_slice(ja_json).context("日本の市区町村データを読めません")?;
    let japan: Vec<(String, Vec<String>)> = PREFECTURES
        .iter()
        .filter_map(|p| ja.get(*p).map(|c| (p.to_string(), c.clone())))
        .collect();
    if japan.len() != 47 {
        bail!("日本の都道府県が47件そろっていません({}件)", japan.len());
    }
    Ok(RegionData {
        countries,
        admin1,
        cities: finish(raw),
        cities_no_admin: finish(raw_no_admin),
        japan,
    })
}

impl RegionData {
    pub fn country(&self, code: &str) -> Option<&CountryRec> {
        self.countries
            .iter()
            .find(|c| c.code.eq_ignore_ascii_case(code))
    }

    /// 都道府県・州の一覧、または(州を持たない国では)都市の一覧。
    pub fn top_level(&self, country: &str) -> Children {
        let cc = country.to_ascii_uppercase();
        if cc == "JP" {
            return Children {
                level: "region".into(),
                label: "都道府県".into(),
                items: self
                    .japan
                    .iter()
                    .map(|(p, _)| RegionItem {
                        code: p.clone(),
                        name: p.clone(),
                    })
                    .collect(),
            };
        }
        if let Some(items) = self.admin1.get(&cc) {
            return Children {
                level: "region".into(),
                label: region_label(&cc).into(),
                items: items.clone(),
            };
        }
        Children {
            level: "city".into(),
            label: "市・都市".into(),
            items: self.cities_no_admin.get(&cc).cloned().unwrap_or_default(),
        }
    }

    /// 都道府県・州の下の市区町村・都市。`parent` は `top_level` の項目の code。
    pub fn cities_of(&self, country: &str, parent: &str) -> Children {
        let cc = country.to_ascii_uppercase();
        if cc == "JP" {
            let items = self
                .japan
                .iter()
                .find(|(p, _)| p == parent)
                .map(|(_, c)| {
                    c.iter()
                        .map(|n| RegionItem {
                            code: n.clone(),
                            name: n.clone(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            return Children {
                level: "city".into(),
                label: "市区町村".into(),
                items,
            };
        }
        Children {
            level: "city".into(),
            label: "市・都市".into(),
            items: self
                .cities
                .get(&format!("{cc}.{parent}"))
                .cloned()
                .unwrap_or_default(),
        }
    }

    /// 都道府県・州の表示名(code から)。無ければ None。
    pub fn region_name(&self, country: &str, code: &str) -> Option<String> {
        self.top_level(country)
            .items
            .into_iter()
            .find(|i| i.code == code)
            .map(|i| i.name)
    }

    /// 都市の表示名が、その州・地方の一覧にあるか。
    pub fn has_city(&self, country: &str, parent: Option<&str>, name: &str) -> bool {
        let list = match parent {
            Some(p) => self.cities_of(country, p),
            None => self.top_level(country),
        };
        list.items.iter().any(|i| i.name == name || i.code == name)
    }
}

/// 国ごとの「州・地方」の呼び方。
fn region_label(cc: &str) -> &'static str {
    match cc {
        "US" | "AU" | "BR" | "MX" | "IN" | "NG" | "MY" | "AT" | "DE" => "州",
        "CN" => "省・自治区",
        "KR" => "道・広域市",
        "GB" => "地域",
        "CA" => "州・準州",
        "FR" | "IT" | "ES" => "地域",
        "RU" => "地方",
        "TW" | "VN" | "TH" | "ID" | "PH" => "県・州",
        _ => "州・地方",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_zip() -> Vec<u8> {
        use std::io::Write;
        let mut buf = Vec::new();
        {
            let mut z = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            z.start_file("cities15000.txt", zip::write::SimpleFileOptions::default())
                .unwrap();
            // id, name, ascii, alt, lat, lon, fclass, fcode, cc, cc2, admin1, admin2, admin3, admin4, population
            for (id, name, cc, a1, pop) in [
                (1, "Los Angeles", "US", "CA", 3898747),
                (2, "San Diego", "US", "CA", 1386932),
                (3, "Reykjavik", "IS", "", 118326),
            ] {
                writeln!(z, "{id}\t{name}\t{name}\t\t0\t0\tP\tPPL\t{cc}\t\t{a1}\t\t\t\t{pop}\t0\t0\ttz\t2020-01-01").unwrap();
            }
            z.finish().unwrap();
        }
        buf
    }

    #[test]
    fn parses_hierarchy_and_special_cases() {
        let mut country_txt = String::from("#header\n");
        for i in 0..210 {
            country_txt.push_str(&format!(
                "X{i}\tXX{i}\t0\t\tCountry{i}\tCap\t1\t1\tAS\t.x\tXXX\tCur\t1\t\t\ten\t0\t\t\n"
            ));
        }
        country_txt.push_str("US\tUSA\t840\tUS\tUnited States\tWashington\t1\t1\tNA\t.us\tUSD\tDollar\t1\t\t\ten-US,es-US\t6252001\t\t\n");
        let admin1 = "US.CA\tCalifornia\tCalifornia\t5332921\nUS.AK\tAlaska\tAlaska\t5879092\n";
        let mut ja = serde_json::Map::new();
        for p in PREFECTURES {
            ja.insert(
                p.to_string(),
                serde_json::json!([format!("{p}のA市"), format!("{p}のB町")]),
            );
        }
        let d = parse(
            country_txt.as_bytes(),
            admin1.as_bytes(),
            &tiny_zip(),
            serde_json::to_string(&ja).unwrap().as_bytes(),
        )
        .unwrap();
        assert!(d
            .country("us")
            .is_some_and(|c| c.languages.starts_with("en-US")));
        // 州は名前順、都市は人口の多い順
        let top = d.top_level("US");
        assert_eq!(
            top.items
                .iter()
                .map(|i| i.name.as_str())
                .collect::<Vec<_>>(),
            ["Alaska", "California"]
        );
        assert_eq!(top.label, "州");
        let cities = d.cities_of("US", "CA");
        assert_eq!(
            cities
                .items
                .iter()
                .map(|i| i.name.as_str())
                .collect::<Vec<_>>(),
            ["Los Angeles", "San Diego"]
        );
        // 州を持たない国は、都市が直接並ぶ
        let is = d.top_level("IS");
        assert_eq!(
            (is.level.as_str(), is.items[0].name.as_str()),
            ("city", "Reykjavik")
        );
        // 日本は都道府県が JIS 順・日本語名、市区町村は日本語
        let jp = d.top_level("jp");
        assert_eq!(
            (
                jp.label.as_str(),
                jp.items[0].name.as_str(),
                jp.items[12].name.as_str()
            ),
            ("都道府県", "北海道", "東京都")
        );
        assert_eq!(d.cities_of("JP", "東京都").items[0].name, "東京都のA市");
        assert!(d.has_city("JP", Some("東京都"), "東京都のB町"));
        assert!(!d.has_city("JP", Some("東京都"), "存在しない市"));
        assert!(d.has_city("US", Some("CA"), "San Diego"));
    }
}
