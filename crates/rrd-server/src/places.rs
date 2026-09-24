//! 調べる「場所」(国全体・都道府県/州・市区町村/都市)と、「知りたい情報」(特産品・温泉・宿泊など)。
//!
//! 場所は3段階で、どの階層も単独で選べる(国全体だけ、都道府県だけ、市区町村だけ、それらの組み合わせ)。
//! 検索の地域・言語(`gl`/`hl`)は国から決める。国の一覧は世界の全ての国(GeoNames)で、
//! 検索の設定が確かめてある主要国(`research::COUNTRIES`)はその設定を使い、それ以外は国コードと主な言語から作る。

use anyhow::{anyhow, bail, Result};

use crate::languages;
use crate::regions::RegionData;
use crate::research::COUNTRIES;

/// 画面から受け取る場所。`country` は国コード(ISO 3166、例: "JP")。
#[derive(Clone, Debug, Default)]
pub struct Place {
    pub country: String,
    /// 都道府県・州(`regions` の code)
    pub region: Option<String>,
    /// 市区町村・都市(名前)
    pub city: Option<String>,
}

/// 検索に使う形にした場所。
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    /// 表示用(例: "日本 › 東京都 › 渋谷区")
    pub label: String,
    pub country_en: String,
    pub country_ja: String,
    pub gl: String,
    pub hl: String,
    /// テーマ・知りたい情報の翻訳先の言語コード(`languages::LANGUAGES` のもの)
    pub lang: &'static str,
    /// 検索語に付ける場所の言葉(国全体なら国名)
    pub place_text: String,
    pub whole_country: bool,
}

/// 「知りたい情報」。`ja` と `en` は固定の検索語、それ以外の言語は AI に翻訳させる(`ai_phrase` が元)。
pub struct Topic {
    pub id: &'static str,
    pub label: &'static str,
    pub ja: &'static str,
    pub en: &'static str,
}

pub const TOPICS: &[Topic] = &[
    Topic {
        id: "specialty",
        label: "特産品・お土産",
        ja: "特産品 お土産 名産",
        en: "local specialty products souvenirs",
    },
    Topic {
        id: "sightseeing",
        label: "観光地・名所",
        ja: "観光地 名所 おすすめ",
        en: "tourist attractions sightseeing spots",
    },
    Topic {
        id: "scenic",
        label: "自然・景勝地(富士五湖など)",
        ja: "湖 自然 景勝地 絶景",
        en: "lakes scenery scenic spots nature",
    },
    Topic {
        id: "mountain",
        label: "山・登山(富士山など)",
        ja: "登山 山 山小屋 予約 アクセス バス",
        en: "mountain hiking climbing mountain huts access",
    },
    Topic {
        id: "onsen",
        label: "温泉",
        ja: "温泉 日帰り温泉 旅館",
        en: "hot springs onsen spa",
    },
    Topic {
        id: "gourmet",
        label: "おいしい食事・グルメ",
        ja: "グルメ おいしい 名物 ご当地",
        en: "best local food restaurants specialties",
    },
    Topic {
        id: "budget_hotel",
        label: "格安ホテル(高速道路沿い・東横INのような)",
        ja: "格安ホテル 東横INのようなリーズナブルなホテル 高速道路沿い",
        en: "cheap budget hotels affordable chain hotels near the highway",
    },
    Topic {
        id: "business_hotel",
        label: "ビジネスホテル",
        ja: "ビジネスホテル",
        en: "business hotels",
    },
    Topic {
        id: "pension",
        label: "ペンション・民宿(郊外)",
        ja: "ペンション 民宿 郊外の宿",
        en: "pension guesthouse B&B countryside inn",
    },
    Topic {
        id: "shrine_temple",
        label: "神社仏閣",
        ja: "神社 寺 仏閣 参拝",
        en: "shrines temples religious sites",
    },
    Topic {
        id: "kimono",
        label: "着物体験",
        ja: "着物体験 レンタル",
        en: "kimono experience rental",
    },
    Topic {
        id: "calligraphy",
        label: "書道体験",
        ja: "書道体験",
        en: "calligraphy experience",
    },
    Topic {
        id: "tea_ceremony",
        label: "茶道体験",
        ja: "茶道体験 お茶",
        en: "tea ceremony experience",
    },
    Topic {
        id: "job_fulltime",
        label: "求人: 正社員",
        ja: "正社員 求人 募集",
        en: "full-time job openings",
    },
    Topic {
        id: "job_parttime",
        label: "求人: アルバイト・パート",
        ja: "アルバイト パート 求人",
        en: "part-time job openings",
    },
    Topic {
        id: "job_freelance",
        label: "求人: フリーランス(プログラマー・IT案件)",
        ja: "フリーランス プログラマー 案件 IT",
        en: "freelance programmer IT project jobs",
    },
    Topic {
        id: "it_training",
        label: "無料のIT研修・転職エージェント",
        ja: "無料 IT研修 転職エージェント 無料",
        en: "free IT training and free career change agents",
    },
    Topic {
        id: "old_industry",
        label: "昔からある産業・伝統産業",
        ja: "伝統産業 地場産業 老舗",
        en: "traditional established local industries",
    },
    Topic {
        id: "growth_industry",
        label: "伸びている産業",
        ja: "成長産業 伸びている業界",
        en: "growing industries",
    },
    Topic {
        id: "cutting_edge",
        label: "最先端企業・スタートアップ",
        ja: "最先端 企業 スタートアップ ベンチャー",
        en: "cutting-edge companies startups",
    },
    Topic {
        id: "agriculture",
        label: "農業情報",
        ja: "農業 農家 農作物",
        en: "agriculture farming",
    },
    Topic {
        id: "forestry",
        label: "林業",
        ja: "林業 森林 林業会社",
        en: "forestry industry",
    },
    Topic {
        id: "timber",
        label: "材木・木材加工業者",
        ja: "製材 木材加工 材木店",
        en: "lumber timber processing companies",
    },
    Topic {
        id: "construction",
        label: "建設会社",
        ja: "建設会社 ゼネコン 施工",
        en: "construction companies",
    },
    Topic {
        id: "builder",
        label: "工務店",
        ja: "工務店 注文住宅 リフォーム",
        en: "local home builders contractors",
    },
    Topic {
        id: "realestate",
        label: "不動産会社",
        ja: "不動産会社 物件 売買 賃貸",
        en: "real estate agencies",
    },
    Topic {
        id: "manufacturer",
        label: "製造業者・メーカー",
        ja: "製造業 メーカー 工場",
        en: "manufacturers factories",
    },
    Topic {
        id: "products",
        label: "製品の紹介とメーカー",
        ja: "製品 紹介 メーカー おすすめ",
        en: "product introductions and their makers",
    },
    Topic {
        id: "sake",
        label: "日本酒の蔵元・酒造",
        ja: "日本酒 酒蔵 蔵元 酒造",
        en: "sake breweries",
    },
    Topic {
        id: "wine",
        label: "ワイン醸造所(ワイナリー)",
        ja: "ワイン ワイナリー 醸造所",
        en: "wineries wine producers",
    },
    Topic {
        id: "whisky",
        label: "ウイスキー蒸溜所",
        ja: "ウイスキー 蒸溜所 ディスティラリー",
        en: "whisky distilleries",
    },
    Topic {
        id: "beer",
        label: "ビール醸造所・クラフトビール",
        ja: "ビール 醸造所 クラフトビール ブルワリー",
        en: "breweries craft beer",
    },
    Topic {
        id: "nonalcohol",
        label: "ノンアルコールビール・飲料メーカー",
        ja: "ノンアルコールビール ノンアルコール飲料 メーカー",
        en: "non-alcoholic beer and beverage makers",
    },
    Topic {
        id: "auto",
        label: "自動車産業",
        ja: "自動車産業 自動車メーカー 部品",
        en: "automotive industry car makers suppliers",
    },
    Topic {
        id: "aircraft",
        label: "航空機産業",
        ja: "航空機産業 航空宇宙 メーカー",
        en: "aircraft aerospace industry",
    },
    Topic {
        id: "defense",
        label: "防衛産業",
        ja: "防衛産業 防衛装備 メーカー",
        en: "defense industry contractors",
    },
    Topic {
        id: "food_processing",
        label: "食料(加工)産業",
        ja: "食品加工 食品メーカー 加工食品",
        en: "food processing industry",
    },
    Topic {
        id: "defense_space",
        label: "防衛: 日本製の人工衛星・AI衛星(軌道上の画像化など)",
        ja: "日本製 AI衛星 人工衛星 軌道上 画像化",
        en: "Japanese made AI satellites artificial satellites on-orbit imaging",
    },
    Topic {
        id: "defense_missile",
        label: "防衛: ミサイル・迎撃・レールガン・レーザー(ICBMなど)",
        ja: "ICBM 大陸間弾道ミサイル 迎撃 防衛 レールガン レーザー",
        en: "ICBM missile defense interception railgun laser weapons",
    },
    Topic {
        id: "factory_tour",
        label: "見学: 工場見学",
        ja: "工場見学 予約 無料",
        en: "factory tours visitor tours booking",
    },
    Topic {
        id: "company_tour",
        label: "見学: 会社見学・企業訪問",
        ja: "会社見学 企業見学 訪問 受け入れ",
        en: "company visits corporate tours for visitors",
    },
    Topic {
        id: "aquaculture_tour",
        label: "見学: 魚介類の養殖・漁業体験",
        ja: "養殖 見学 漁業体験 水産",
        en: "aquaculture fish farm visits fishing experience",
    },
    Topic {
        id: "strawberry_farm",
        label: "見学: いちご農家・いちご狩り",
        ja: "いちご農家 いちご狩り 見学",
        en: "strawberry farms strawberry picking",
    },
    Topic {
        id: "farm_experience",
        label: "体験: 農業体験",
        ja: "農業体験 農家 収穫体験 受け入れ",
        en: "farm experience agritourism harvest",
    },
    Topic {
        id: "forestry_experience",
        label: "体験: 林業体験",
        ja: "林業体験 森林 伐採 体験 見学",
        en: "forestry experience forest work visits",
    },
    Topic {
        id: "industry_tour",
        label: "見学: 産業別の見学(酒蔵・茶畑・酪農・職人の工房など)",
        ja: "産業観光 見学 酒蔵 茶畑 酪農 工房 職人",
        en: "industrial tourism visits breweries tea farms dairy workshops artisans",
    },
    Topic {
        id: "sake_tour",
        label: "見学: 日本酒の酒蔵・蔵見学",
        ja: "酒蔵 見学 蔵開き 試飲 日本酒",
        en: "sake brewery tours tasting",
    },
    Topic {
        id: "whisky_tour",
        label: "見学: ウイスキー蒸溜所見学",
        ja: "ウイスキー 蒸溜所 見学 ツアー 試飲",
        en: "whisky distillery tours tasting",
    },
    Topic {
        id: "wine_tour",
        label: "見学: ワイナリー・ぶどう農家見学",
        ja: "ワイナリー 見学 ぶどう畑 ワイン 試飲 ぶどう農家",
        en: "winery tours vineyard visits grape farms tasting",
    },
    Topic {
        id: "beer_tour",
        label: "見学: ビール工場・醸造所見学",
        ja: "ビール工場 見学 醸造所 ツアー 試飲",
        en: "brewery tours beer factory visits tasting",
    },
    Topic {
        id: "producer_tour",
        label: "見学: 生産農家・生産者(お茶・果樹・酪農・醸造原料など)",
        ja: "生産者 農家 見学 直売 収穫 体験 茶園 果樹園 牧場",
        en: "farm and producer visits tea plantation orchard dairy ranch",
    },
    Topic {
        id: "similar_tour",
        label: "見学: 焼酎・泡盛・味噌・醤油・チーズ・お茶などの製造所",
        ja: "焼酎 泡盛 味噌 醤油 チーズ 製茶 工場 見学",
        en: "tours of shochu awamori miso soy sauce cheese tea factories",
    },
];

/// 知りたい情報の検索語。日本語・英語は固定、それ以外は翻訳結果(無ければ英語)。
pub fn phrase_for(
    topic: &Topic,
    lang: &str,
    translated: &std::collections::HashMap<String, std::collections::HashMap<String, String>>,
) -> String {
    match lang {
        "ja" => topic.ja.to_string(),
        "en" => topic.en.to_string(),
        other => translated
            .get(other)
            .and_then(|m| m.get(topic.id))
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| topic.en.to_string()),
    }
}

pub fn topic(id: &str) -> Option<&'static Topic> {
    TOPICS.iter().find(|t| t.id == id)
}

/// 国コード → 主要国の設定。`gl` は Google の地域コードで、ISO とほぼ同じ(英国だけ "uk")。
fn curated(
    iso2: &str,
) -> Option<&'static (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
)> {
    COUNTRIES.iter().find(|c| {
        let code = if c.1 == "uk" { "GB" } else { c.1 };
        code.eq_ignore_ascii_case(iso2)
    })
}

/// GeoNames の言語(例: "zh-CN,yue,wuu")から、検索の言語コードと、翻訳先の言語コードを作る。
fn language_from(languages_field: &str) -> (String, &'static str) {
    let primary = languages_field
        .split(',')
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("en");
    let hl = primary.to_ascii_lowercase();
    let base = hl.split('-').next().unwrap_or("en").to_string();
    let lang = if base == "zh" {
        if hl.contains("tw") || hl.contains("hk") || hl.contains("hant") {
            "zh-Hant"
        } else {
            "zh-Hans"
        }
    } else {
        languages::find(primary)
            .or_else(|| languages::find(&base))
            .map_or("en", |l| l.0)
    };
    (hl, lang)
}

pub fn resolve(p: &Place, regions: &RegionData) -> Result<Target> {
    let country = regions
        .country(&p.country)
        .ok_or_else(|| anyhow!("国コードが正しくありません: {}", p.country))?;
    let (gl, hl, country_ja, lang): (String, String, String, &'static str) =
        match curated(&country.code) {
            Some(&(_, gl, hl, ja, lang)) => {
                let l = languages::find(lang).map_or("en", |x| x.0);
                (gl.to_string(), hl.to_string(), ja.to_string(), l)
            }
            None => {
                let (hl, lang) = language_from(&country.languages);
                (
                    country.code.to_ascii_lowercase(),
                    hl,
                    country.name.clone(),
                    lang,
                )
            }
        };
    let is_ja = hl.starts_with("ja");
    // 階層の検証(存在しない都道府県・市区町村は受け付けない)
    let region_name =
        match &p.region {
            Some(code) => Some(regions.region_name(&country.code, code).ok_or_else(|| {
                anyhow!("{} に、その都道府県・州はありません: {code}", country.name)
            })?),
            None => None,
        };
    if let Some(city) = &p.city {
        if city.chars().count() > 80 {
            bail!("市区町村・都市の名前が長すぎます");
        }
        if !regions.has_city(&country.code, p.region.as_deref(), city) {
            bail!(
                "{} に、その市区町村・都市はありません: {city}",
                country.name
            );
        }
    }
    let city_name = p.city.as_ref().map(|c| {
        // 都市は GeoNames の geonameid(数字)で渡される場合があるので、表示名に直す
        let mut all = regions.top_level(&country.code).items;
        if let Some(r) = p.region.as_deref() {
            all.extend(regions.cities_of(&country.code, r).items);
        }
        all.iter()
            .find(|i| &i.code == c)
            .map_or_else(|| c.clone(), |i| i.name.clone())
    });
    let mut label = country_ja.clone();
    for part in [&region_name, &city_name].into_iter().flatten() {
        label.push_str(" › ");
        label.push_str(part);
    }
    let whole_country = region_name.is_none() && city_name.is_none();
    let place_text = if whole_country {
        if is_ja {
            country_ja.clone()
        } else {
            country.name.clone()
        }
    } else if is_ja {
        [region_name.as_deref(), city_name.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        // 外国は「都市, 州, 国」の順(英語表記の地名で、検索エンジンが場所を特定しやすい)
        [
            city_name.as_deref(),
            region_name.as_deref(),
            Some(country.name.as_str()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ")
    };
    Ok(Target {
        label,
        country_en: country.name.clone(),
        country_ja,
        gl,
        hl,
        lang,
        place_text,
        whole_country,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_mapping_for_uncurated_countries() {
        assert_eq!(
            language_from("zh-CN,yue,wuu"),
            ("zh-cn".to_string(), "zh-Hans")
        );
        assert_eq!(language_from("zh-TW,zh"), ("zh-tw".to_string(), "zh-Hant"));
        assert_eq!(
            language_from("en-US,es-US,haw,fr"),
            ("en-us".to_string(), "en")
        );
        assert_eq!(language_from("fr-CH,de-CH,it-CH,rm").1, "fr");
        assert_eq!(language_from("").1, "en");
        assert_eq!(language_from("xxx").1, "en", "一覧に無い言語は英語");
    }

    #[test]
    fn topics_are_unique_and_have_fixed_phrases() {
        for (i, t) in TOPICS.iter().enumerate() {
            assert!(TOPICS[..i].iter().all(|u| u.id != t.id), "重複: {}", t.id);
            assert!(!t.ja.is_empty() && !t.en.is_empty());
        }
        assert!(topic("mountain").is_some() && topic("nope").is_none());
        // 富士五湖・神社仏閣・着物/書道/茶道体験・温泉・各種宿泊が含まれる
        for id in [
            "scenic",
            "mountain",
            "shrine_temple",
            "kimono",
            "calligraphy",
            "tea_ceremony",
            "onsen",
            "budget_hotel",
            "business_hotel",
            "pension",
        ] {
            assert!(topic(id).is_some(), "{id}");
        }
    }

    #[test]
    fn curated_lookup_uses_iso_codes_including_uk() {
        assert_eq!(curated("JP").unwrap().0, "Japan");
        assert_eq!(curated("gb").unwrap().0, "United Kingdom");
        assert!(curated("IS").is_none());
    }
}
