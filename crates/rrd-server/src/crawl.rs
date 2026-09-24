//! 毎朝の自動収集(日本全国の体験・見学・学び・育成の情報)。
//!
//! 毎日 7:00(日本時間)以降の最初の機会に、下の `TOPICS` を「日本全体」と、日替わりの都道府県で検索し、
//! 記事の見出し・要約・URL だけをデータセット `daily_jp_YYYYMMDD` に保存する(AI の分析はしない)。
//! 「場所(日本全体+47都道府県)×知りたい情報」の組を順番に、毎日 `searches_per_day()`(既定20、環境変数
//! `RRD_CRAWL_SEARCHES_PER_DAY` で変更、最大200)組ずつ検索して回る。共有の検索は無料枠が1日100回のため、
//! 既定は少なめにしてある。検索の枠を増やしたら、この値を増やすと全国を早く一巡できる。
//! 保存先は `RRD_DATA_DIR/daily/`。古いものは `KEEP_DAYS` 日ぶんだけ残す。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use crate::places::Place;
use crate::research;
use crate::schema::AppState;

/// 毎朝集める「知りたい情報」
pub const TOPICS: &[&str] = &[
    "stream_fishing",
    "shellfish_tour",
    "mushroom_picking",
    "mushroom_factory",
    "pc_class",
    "claude_class",
    "factory_tour",
    "company_tour",
    "aquaculture_tour",
    "strawberry_farm",
    "farm_experience",
    "forestry_experience",
    "industry_tour",
    "sake_tour",
    "whisky_tour",
    "wine_tour",
    "beer_tour",
    "producer_tour",
    "similar_tour",
    "construction_training",
    "construction_site_tour",
    "carpenter",
    "carpenter_site_tour",
    "facility_mgmt_training",
    "facility_mgmt_fire",
    "job_construction",
    "job_facility",
];
const DEFAULT_SEARCHES_PER_DAY: usize = 20;
const KEEP_DAYS: usize = 7;

static RUNNING: AtomicBool = AtomicBool::new(false);
/// 最後に収集を試みた日(通算日)。失敗しても同じ日に何度も検索し直さない(検索の無料枠を守る)
static LAST_TRY_DAY: AtomicU64 = AtomicU64::new(0);

fn searches_per_day() -> usize {
    std::env::var("RRD_CRAWL_SEARCHES_PER_DAY")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .map_or(DEFAULT_SEARCHES_PER_DAY, |n| n.clamp(1, 200))
}

fn day_index(unix: u64) -> u64 {
    (unix + 9 * 3600) / 86_400
}

fn dir(st: &AppState) -> PathBuf {
    st.data_dir.join("daily")
}

fn today(unix: u64) -> String {
    let (y, m, d, _) = crate::market::jst(unix);
    format!("daily_jp_{y:04}{m:02}{d:02}")
}

/// 今日のぶんがまだなら true
pub fn is_due(st: &AppState) -> bool {
    if RUNNING.load(Ordering::SeqCst) {
        return false;
    }
    let now = crate::market::now_unix();
    if LAST_TRY_DAY.load(Ordering::SeqCst) == day_index(now) {
        return false;
    }
    let name = today(now);
    !dir(st).join(format!("{name}.csv")).exists()
}

/// 保存済みの最新の収集結果を、起動時に読み込む(再起動しても分析に使えるようにする)。
pub fn load_latest(st: &Arc<AppState>) {
    let Ok(rd) = std::fs::read_dir(dir(st)) else {
        return;
    };
    let mut files: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "csv"))
        .collect();
    files.sort();
    for p in files.iter().rev().take(KEEP_DAYS) {
        let (Some(name), Ok(df)) = (
            p.file_stem().and_then(|s| s.to_str()).map(String::from),
            rrd_core::csv::read_csv(p),
        ) else {
            continue;
        };
        if let Ok(mut m) = st.datasets.write() {
            m.insert(name, df);
        }
    }
}

/// 今日検索する(場所, 知りたい情報)の組。場所は 0=日本全体、1〜=都道府県(`prefs` の順)。
/// 「場所 × 知りたい情報」を場所順に並べ、毎日 `budget` 組ずつ続きから取る(終わったら最初に戻る)。
fn plan(day_index: u64, budget: usize, n_places: usize) -> Vec<(usize, Vec<&'static str>)> {
    let total = n_places * TOPICS.len();
    if total == 0 {
        return Vec::new();
    }
    let start = (day_index as usize * budget) % total;
    let mut out: Vec<(usize, Vec<&'static str>)> = Vec::new();
    for k in 0..budget.min(total) {
        let idx = (start + k) % total;
        let (place, topic) = (idx / TOPICS.len(), TOPICS[idx % TOPICS.len()]);
        match out.last_mut() {
            Some((p, v)) if *p == place => v.push(topic),
            _ => out.push((place, vec![topic])),
        }
    }
    out
}

pub async fn run_daily(st: Arc<AppState>) {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    LAST_TRY_DAY.store(day_index(crate::market::now_unix()), Ordering::SeqCst);
    if let Err(e) = collect(&st).await {
        eprintln!("realdata.pro: 毎朝の自動収集に失敗: {e:#}");
    }
    RUNNING.store(false, Ordering::SeqCst);
}

async fn collect(st: &Arc<AppState>) -> anyhow::Result<()> {
    let Some(regions) = st.regions.read().ok().and_then(|g| g.clone()) else {
        return Ok(()); // 地名データが未読み込み。次の巡回(5分後)にやり直す
    };
    let now = crate::market::now_unix();
    let prefs: Vec<String> = regions
        .top_level("JP")
        .items
        .into_iter()
        .map(|i| i.code)
        .collect();
    let mut places: Vec<(Place, Vec<String>)> = Vec::new();
    for (idx, topics) in plan(day_index(now), searches_per_day(), prefs.len() + 1) {
        let region = if idx == 0 {
            None
        } else {
            prefs.get(idx - 1).cloned()
        };
        places.push((
            Place {
                country: "JP".into(),
                region,
                city: None,
            },
            topics.iter().map(|s| s.to_string()).collect(),
        ));
    }
    let mut merged: Option<String> = None;
    let (mut ok, mut failed) = (0usize, 0usize);
    for (place, topics) in places {
        let opt = research::Options {
            theme: String::new(),
            places: vec![place],
            topics,
            search_languages: Vec::new(),
            translate_items: false,
            per_country: 3,
            include_news: false,
            include_github: false,
            include_youtube: false,
            languages: Vec::new(),
            analyze: false,
        };
        match research::run(&st.http, &st.llm_base, &regions, opt).await {
            Ok(out) => {
                ok += 1;
                let csv = rrd_core::csv::to_csv_string(&out.df);
                match &mut merged {
                    None => merged = Some(csv),
                    Some(m) => {
                        // 見出し行を除いて連結する
                        for line in csv.lines().skip(1) {
                            m.push_str(line);
                            m.push('\n');
                        }
                    }
                }
            }
            Err(e) => {
                failed += 1;
                let msg = format!("{e:#}");
                eprintln!(
                    "realdata.pro: 毎朝の自動収集(1か所)に失敗: {}",
                    msg.chars().take(200).collect::<String>()
                );
                if msg.contains("quota") {
                    break; // 検索の無料枠が尽きている。残りの場所は明日にする
                }
            }
        }
    }
    let Some(csv) = merged else {
        anyhow::bail!("どの場所も収集できませんでした");
    };
    let df = rrd_core::csv::read_csv_str(&csv)?;
    let name = today(now);
    let d = dir(st);
    std::fs::create_dir_all(&d)?;
    std::fs::write(d.join(format!("{name}.csv")), &csv)?;
    if let Ok(mut m) = st.datasets.write() {
        m.insert(name.clone(), df);
    }
    if let Some(store) = &st.store {
        if let Err(e) = store.save(&name, &csv, now as i64).await {
            eprintln!("realdata.pro: 自動収集の保存(aruaru-db)に失敗: {e:#}");
        }
    }
    prune(st);
    eprintln!("realdata.pro: 毎朝の自動収集を保存しました({name}、{ok}か所成功・{failed}か所失敗)");
    Ok(())
}

/// 古い日のデータ(ファイルとメモリ上)を消す
fn prune(st: &Arc<AppState>) {
    let Ok(rd) = std::fs::read_dir(dir(st)) else {
        return;
    };
    let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    files.sort();
    let extra = files.len().saturating_sub(KEEP_DAYS);
    for p in files.into_iter().take(extra) {
        let stem = p.file_stem().and_then(|s| s.to_str()).map(String::from);
        let _ = std::fs::remove_file(&p);
        if let (Some(s), Ok(mut m)) = (stem, st.datasets.write()) {
            m.remove(&s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_uses_the_budget_and_covers_everything_eventually() {
        let n_places = 48;
        let total = n_places * TOPICS.len();
        let mut seen = std::collections::HashSet::new();
        let days = total.div_ceil(20) as u64;
        for day in 0..days {
            let p = plan(day, 20, n_places);
            assert_eq!(p.iter().map(|(_, v)| v.len()).sum::<usize>(), 20);
            for (place, topics) in p {
                for t in topics {
                    seen.insert((place, t));
                }
            }
        }
        assert_eq!(seen.len(), total, "毎日20組ずつで、全ての組を一巡する");
        assert!(plan(0, 20, 0).is_empty());
    }

    #[test]
    fn all_crawl_topics_exist() {
        for id in TOPICS {
            assert!(crate::places::topic(id).is_some(), "{id}");
        }
    }
}
