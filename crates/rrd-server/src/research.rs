//! 世界リサーチ: テーマを入れるだけで、世界情勢・各国のマーケティング情報を集め、
//! 解析・分析し、提案まで出す。
//!
//! 流れ:
//! 1. テーマを各国の言語に翻訳する(aruaru-llm の無料 AI)。翻訳に失敗したら元のテーマのまま使う。
//! 2. 国ごとに、現地の地域・言語で Google 検索を行う。国別の主要ニュース(aruaru-llm、3時間キャッシュ)も、
//!    選んだ場合は集める。GitHub / YouTube は、選んだ場合に全体として1回だけ検索する。
//! 3. 集めた記事を1つのデータセットにする(ほかの分析タブでもそのまま使える)。
//! 4. AI に分析させる。出力は JSON(要約・トレンド・機会・リスク・提案・国別の論調)。
//!    **根拠は記事の番号で答えさせ、URL はサーバー側で番号から付ける**。
//!    AI が存在しない URL を作ることはできない。存在しない番号は捨てる。
//! 5. 2つ目以降の言語は、1つ目の言語の結果を翻訳して作る(言語ごとに内容が食い違わないようにする)。

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use rrd_core::DataFrame;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use futures::StreamExt;

use crate::explain::complete;
use crate::languages;
use crate::places::{self, Place, Target, Topic};
use crate::regions::RegionData;

/// リサーチ対象にできる国・地域。
/// (aruaru-llm の国別ニュースの国名, Google の gl, hl, 日本語名, 翻訳先の言語コード)
pub const COUNTRIES: &[(&str, &str, &str, &str, &str)] = &[
    ("Japan", "jp", "ja", "日本", "ja"),
    ("United States", "us", "en", "アメリカ", "en"),
    ("United Kingdom", "uk", "en", "イギリス", "en"),
    ("China", "cn", "zh-cn", "中国", "zh-Hans"),
    ("Taiwan", "tw", "zh-tw", "台湾", "zh-Hant"),
    ("South Korea", "kr", "ko", "韓国", "ko"),
    ("India", "in", "en", "インド", "en"),
    ("Indonesia", "id", "id", "インドネシア", "id"),
    ("Vietnam", "vn", "vi", "ベトナム", "vi"),
    ("Thailand", "th", "th", "タイ", "th"),
    ("Philippines", "ph", "en", "フィリピン", "en"),
    ("Malaysia", "my", "en", "マレーシア", "en"),
    ("Singapore", "sg", "en", "シンガポール", "en"),
    ("Australia", "au", "en", "オーストラリア", "en"),
    ("Germany", "de", "de", "ドイツ", "de"),
    ("France", "fr", "fr", "フランス", "fr"),
    ("Italy", "it", "it", "イタリア", "it"),
    ("Spain", "es", "es", "スペイン", "es"),
    ("Netherlands", "nl", "nl", "オランダ", "nl"),
    ("Poland", "pl", "pl", "ポーランド", "pl"),
    ("Russia", "ru", "ru", "ロシア", "ru"),
    ("Ukraine", "ua", "uk", "ウクライナ", "uk"),
    ("Turkey", "tr", "tr", "トルコ", "tr"),
    ("Saudi Arabia", "sa", "ar", "サウジアラビア", "ar"),
    ("United Arab Emirates", "ae", "ar", "アラブ首長国連邦", "ar"),
    ("Israel", "il", "en", "イスラエル", "en"),
    ("Egypt", "eg", "ar", "エジプト", "ar"),
    ("Nigeria", "ng", "en", "ナイジェリア", "en"),
    ("South Africa", "za", "en", "南アフリカ", "en"),
    ("Kenya", "ke", "en", "ケニア", "en"),
    ("Brazil", "br", "pt", "ブラジル", "pt-BR"),
    ("Mexico", "mx", "es", "メキシコ", "es"),
    ("Argentina", "ar", "es", "アルゼンチン", "es"),
    ("Canada", "ca", "en", "カナダ", "en"),
    ("Myanmar", "mm", "en", "ミャンマー", "my"),
];

pub const MAX_PLACES: usize = 6;
/// 1回のリサーチで行う Web 検索の上限(場所 × 検索言語 × (テーマ + 知りたい情報))。検索 API の無料枠を守るため。
const MAX_SEARCHES: usize = 30;
/// 現地語に加えて追加できる検索言語の数。
const MAX_EXTRA_LANGS: usize = 3;
const MAX_PER_COUNTRY: u8 = 10;
/// AI に渡す資料の上限文字数(aruaru-llm の上限 20,000 文字に余裕を持たせる)。
const MAX_BRIEF_CHARS: usize = 14_000;

pub struct Options {
    /// テーマ(空でもよい。その場合は知りたい情報だけで検索する)
    pub theme: String,
    /// 調べる場所(国全体・都道府県/州・市区町村/都市)
    pub places: Vec<Place>,
    /// 知りたい情報(places::TOPICS の id)
    pub topics: Vec<String>,
    /// 現地語に加えて検索に使う言語(世界の約130言語から)
    pub search_languages: Vec<String>,
    /// 集めた記事の見出し・要約を、先頭のレポート言語へ翻訳して併記する
    pub translate_items: bool,
    pub per_country: u8,
    pub include_news: bool,
    pub include_github: bool,
    pub include_youtube: bool,
    pub languages: Vec<String>,
}

/// 収集した1件。
#[derive(Clone, Debug)]
pub struct Item {
    /// 場所の表示名(例: 日本 › 東京都 › 渋谷区)
    pub country: String,
    /// テーマ / 知りたい情報の名前 / 国の一般情勢
    pub topic: String,
    pub source: &'static str,
    pub query: String,
    /// 関連性の判定に使う、この検索の主要な語(テーマまたは知りたい情報の検索語。場所は含めない)
    pub key: String,
    pub title_tr: Option<String>,
    pub snippet_tr: Option<String>,
    pub title: String,
    pub snippet: String,
    pub url: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Finding {
    pub title: String,
    pub detail: String,
    pub countries: Vec<String>,
    pub evidence: Vec<usize>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Proposal {
    pub action: String,
    pub why: String,
    pub priority: String,
    pub evidence: Vec<usize>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Sentiment {
    pub country: String,
    /// -2(とても否定的)〜 +2(とても肯定的)
    pub score: f64,
    pub reason: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Analysis {
    pub summary: String,
    pub trends: Vec<Finding>,
    pub opportunities: Vec<Finding>,
    pub risks: Vec<Finding>,
    pub proposals: Vec<Proposal>,
    pub sentiment: Vec<Sentiment>,
}

pub struct Report {
    pub lang: String,
    pub language_name: String,
    pub analysis: Analysis,
    pub provider: Option<String>,
}

pub struct Outcome {
    pub items: Vec<Item>,
    pub df: DataFrame,
    pub reports: Vec<Report>,
    pub warnings: Vec<String>,
    /// 国ごとに実際に使った検索語(英語の国名, 検索語)
    pub queries: Vec<(String, String)>,
    pub millis: f64,
}

/// AI の回答から最初の JSON オブジェクトを取り出す(```json で囲まれていても可)。
pub fn extract_json(text: &str) -> Option<Json> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&text[start..=end]).ok()
}

async fn search_raw(http: &reqwest::Client, base: &str, body: Json) -> Result<Vec<Json>> {
    let resp = http
        .post(format!("{}/v1/search/raw", base.trim_end_matches('/')))
        .json(&body)
        .timeout(Duration::from_secs(40))
        .send()
        .await
        .context("aruaru-llm に接続できません")?;
    let status = resp.status();
    let j: Json = resp.json().await.context("aruaru-llm の応答を読めません")?;
    if !status.is_success() {
        bail!(
            "{}",
            j.get("error")
                .and_then(Json::as_str)
                .unwrap_or("検索に失敗しました")
        );
    }
    Ok(j.get("results")
        .and_then(Json::as_array)
        .cloned()
        .unwrap_or_default())
}

async fn country_news(http: &reqwest::Client, base: &str, name: &str) -> Result<Vec<Json>> {
    let url = reqwest::Url::parse_with_params(
        &format!("{}/v1/news/for", base.trim_end_matches('/')),
        &[("country", name)],
    )?;
    let j: Json = http
        .get(url)
        .timeout(Duration::from_secs(40))
        .send()
        .await
        .context("aruaru-llm に接続できません")?
        .json()
        .await
        .context("ニュースの応答を読めません")?;
    let items = j
        .get("items")
        .and_then(Json::as_array)
        .cloned()
        .unwrap_or_default();
    if items.is_empty() {
        if let Some(err) = j
            .get("last_error")
            .and_then(Json::as_str)
            .filter(|e| !e.is_empty())
        {
            bail!("{err}");
        }
    }
    Ok(items)
}
fn s(j: &Json, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|k| j.get(*k).and_then(Json::as_str))
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// テーマを各言語に翻訳する。失敗したら空(=元のテーマを使う)。
async fn translate_theme(
    http: &reqwest::Client,
    base: &str,
    theme: &str,
    langs: &[&str],
) -> Result<HashMap<String, String>> {
    let targets: Vec<String> = langs
        .iter()
        .filter_map(|l| languages::find(l).map(|(c, _, n)| format!("\"{c}\" ({n})")))
        .collect();
    let prompt = format!(
        "Translate this market-research theme into ONE natural web-search query per language: 2-5 keywords that a local \
         person would type, no commas, no alternatives, no quotes.\n\
         Theme: {theme}\nLanguages: {}\n\
         Return ONLY a JSON object mapping each language code to the keywords, e.g. {{\"en\": \"...\"}}. No explanations.",
        targets.join(", ")
    );
    let (text, _) = complete(http, base, &prompt).await?;
    let j = extract_json(&text).ok_or_else(|| anyhow!("翻訳結果を読み取れませんでした"))?;
    Ok(j.as_object()
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), single_query(s))))
                .collect()
        })
        .unwrap_or_default())
}

/// 翻訳結果を1つの検索語にする。AI が「A, B, C」のように複数案を返しても最初の1つだけ使う
/// (複数案をそのまま検索すると、検索エンジンが広く解釈して無関係な結果が混ざる。2026-09-24 に実例あり)。
fn single_query(s: &str) -> String {
    let first = s
        .split([',', '、', '，', ';', '/', '|', '\n'])
        .map(str::trim)
        .find(|p| !p.is_empty())
        .unwrap_or("");
    first
        .trim_matches(['"', '\'', '「', '」'])
        .chars()
        .take(120)
        .collect()
}

/// 収集した記事を、番号付きの資料にする(長すぎる場合は均等に間引く)。
/// 検索語から「主要な語」を取り出す(2文字以上。英数字は小文字化)。
/// 需要・市場・世界のような、どのテーマにも付く一般語は除く(これだけ一致した記事は無関係なことが多い)。
fn topic_tokens(q: &str) -> Vec<String> {
    const GENERIC: &[&str] = &[
        "需要",
        "海外",
        "市場",
        "世界",
        "動向",
        "トレンド",
        "分析",
        "demand",
        "global",
        "market",
        "markets",
        "worldwide",
        "international",
        "trend",
        "trends",
        "analysis",
        "industry",
        "business",
        "nachfrage",
        "globale",
        "markt",
        "weltweit",
        "demande",
        "marché",
        "mondial",
        "demanda",
        "mercado",
        "mundial",
        "需求",
        "市场",
        "全球",
        "海外市场",
        "수요",
        "시장",
        "세계",
    ];
    q.split(|c: char| c.is_whitespace() || ",、，。・/|()（）\"'".contains(c))
        .map(|t| t.trim().to_lowercase())
        .filter(|t| t.chars().count() >= 2 && !GENERIC.contains(&t.as_str()))
        .collect()
}

/// 見出しか要約に主要な語を1つ以上含むか。主要な語が1つも無い(一般語だけの)テーマなら常に採用する。
fn is_relevant(it: &Item, tokens: &[String]) -> bool {
    if tokens.is_empty() {
        return true;
    }
    let hay = format!("{} {}", it.title, it.snippet).to_lowercase();
    tokens.iter().any(|t| hay.contains(t.as_str()))
}

fn brief(theme: &str, items: &[Item]) -> String {
    let mut lines: Vec<String> = items
        .iter()
        .enumerate()
        .map(|(i, it)| {
            let snip: String = it.snippet.chars().take(220).collect();
            format!(
                "[{}] {} | {} | {} | {} — {}",
                i + 1,
                it.country,
                it.topic,
                it.source,
                it.title,
                snip
            )
        })
        .collect();
    let mut total: usize = lines.iter().map(|l| l.chars().count() + 1).sum();
    while total > MAX_BRIEF_CHARS && !lines.is_empty() {
        // 各記事の要約を短くしていく(見出しと番号は残す)
        let longest = (0..lines.len())
            .max_by_key(|&i| lines[i].chars().count())
            .unwrap();
        let keep = lines[longest].chars().count() * 3 / 4;
        lines[longest] = lines[longest].chars().take(keep.max(40)).collect();
        total = lines.iter().map(|l| l.chars().count() + 1).sum();
        if lines.iter().all(|l| l.chars().count() <= 40) {
            break;
        }
    }
    let (mut topic, mut context) = (Vec::new(), Vec::new());
    for (line, it) in lines.into_iter().zip(items) {
        if it.source == "news" {
            context.push(line);
        } else {
            topic.push(line);
        }
    }
    let mut out = format!(
        "Research theme / topics: {theme}\nItems (number | place | topic | source | title — snippet):\n{}",
        topic.join("\n")
    );
    if !context.is_empty() {
        out.push_str(&format!(
            "\n\nGeneral national headlines (background on each country's current situation; NOT specifically about the theme):\n{}",
            context.join("\n")
        ));
    }
    out
}

fn analysis_prompt(
    brief: &str,
    lang_code: &str,
    native: &str,
    countries: &[String],
    extra: &str,
) -> String {
    format!(
        "You are a senior global market analyst advising companies of every size (large enterprises, SMEs and \
         first-time founders). Analyze ONLY the collected items below.\n\
         Write every text value naturally in {native} (language code {lang_code}) only; do not mix in words from other \
         languages except proper nouns such as brand or company names.\n\
         Return ONLY one JSON object, no markdown, with exactly these keys:\n\
         {{\"summary\": string (4-6 sentences),\n\
           \"trends\": [{{\"title\": string, \"detail\": string, \"countries\": [string], \"evidence\": [item numbers]}}] (3-5),\n\
           \"opportunities\": [same shape] (2-4),\n\
           \"risks\": [same shape] (2-4),\n\
           \"proposals\": [{{\"action\": string, \"why\": string, \"priority\": \"high\"|\"medium\"|\"low\", \"evidence\": [item numbers]}}] (3-5, concrete and low-cost first),\n\
           \"sentiment\": [{{\"country\": string, \"score\": number from -2 to 2, \"reason\": string}}] for each of: {}}}\n\
         Use the general national headlines only as background on each country's situation (economy, politics, risks); \
         never cite them as evidence about the theme itself.\n\
         Rules: cite evidence ONLY by the item numbers shown in brackets (inside text write them as [n] or [n, m]); never write URLs; do not invent facts or numbers \
         that are not in the items; if the items are insufficient, say so in the summary. Use the place names \
         exactly as listed for the sentiment \"country\" field.\n{extra}\n\n{brief}",
        countries.join(", ")
    )
}

/// Google の表示言語(hl)にする。`languages` の言語コードを小文字にし、中国語だけ地域付きにする。
fn hl_for(code: &str) -> String {
    match code {
        "zh-Hans" => "zh-cn".to_string(),
        "zh-Hant" => "zh-tw".to_string(),
        other => other.to_ascii_lowercase(),
    }
}

enum Kind {
    Web,
    News,
}

/// 1回の検索。
struct Job {
    target: usize,
    topic: Option<&'static Topic>,
    lang_ja: &'static str,
    gl: String,
    hl: String,
    query: String,
    key: String,
    /// その場所の現地語の検索か(追加の言語ではないか)
    is_own: bool,
    kind: Kind,
}

/// 知りたい情報の検索語を、言語ごとに AI に翻訳させる(日本語・英語は固定の検索語を使う)。
async fn translate_topics(
    http: &reqwest::Client,
    base: &str,
    topics: &[&Topic],
    langs: &[&str],
) -> Result<HashMap<String, HashMap<String, String>>> {
    let targets: Vec<String> = langs
        .iter()
        .filter_map(|l| languages::find(l).map(|(c, _, n)| format!("\"{c}\" ({n})")))
        .collect();
    let phrases: Vec<String> = topics
        .iter()
        .map(|t| format!("\"{}\": \"{}\"", t.id, t.en))
        .collect();
    let prompt = format!(
        "Translate each English phrase into natural web-search keywords (2-6 words, no commas, no alternatives) for each language.\n\
         Phrases (id: English): {{{}}}\nLanguages: {}\n\
         Return ONLY a JSON object of the form {{\"<language code>\": {{\"<id>\": \"<keywords>\"}}}}. No explanations.",
        phrases.join(", "),
        targets.join(", ")
    );
    let (text, _) = complete(http, base, &prompt).await?;
    let j = extract_json(&text).ok_or_else(|| anyhow!("翻訳結果を読み取れませんでした"))?;
    let mut out: HashMap<String, HashMap<String, String>> = HashMap::new();
    if let Some(o) = j.as_object() {
        for (lang, m) in o {
            if let Some(mo) = m.as_object() {
                let inner = mo
                    .iter()
                    .filter_map(|(id, v)| v.as_str().map(|s| (id.clone(), single_query(s))))
                    .collect();
                out.insert(lang.clone(), inner);
            }
        }
    }
    Ok(out)
}

/// 知りたい情報にもとづく、AI への追加の指示(場所ごとの見どころの整理、山の安全上の注意)。
fn extra_context(topics: &[&Topic]) -> String {
    if topics.is_empty() {
        return String::new();
    }
    let labels: Vec<&str> = topics.iter().map(|t| t.label).collect();
    let mut s = format!(
        "The collected items include local information per place and topic ({}). For each place, summarize what stands out for each \
         topic (specialties, sights, hot springs, food, lodging, cultural experiences) and turn it into practical proposals for a visitor \
         or a business: itinerary ideas, what to book in advance, budget points and cautions. When lodging topics are present, compare the \
         types (cheap highway hotels, business hotels, countryside pensions).",
        labels.join(", ")
    );
    if topics.iter().any(|t| t.id == "mountain") {
        s.push_str(
            "\nMOUNTAIN SAFETY: mountain climbing is included (for example Mt. Fuji), so always add to \"risks\" and \"proposals\" a safety \
             checklist: reserve buses/taxis and mountain huts in advance, check the official climbing season and route/road closures, \
             prepare for cold even in summer (warm layers such as ski wear, waterproofs), and carry a headlamp and a helmet. Every item that is \
             NOT taken from the collected items must start with \"General advice:\" and have an empty evidence list. Tell the reader to \
             confirm current official information before going.",
        );
    }
    s
}

/// 対象の言語で書かれているように見えるか(見出し・要約を翻訳するかどうかの目安)。
fn looks_like(lang: &str, text: &str) -> bool {
    let letters: Vec<char> = text.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.is_empty() {
        return true;
    }
    let share = |f: fn(char) -> bool| {
        letters.iter().filter(|c| f(**c)).count() as f64 / letters.len() as f64
    };
    match lang {
        "ja" => {
            share(|c| {
                ('\u{3040}'..='\u{30ff}').contains(&c) || ('\u{4e00}'..='\u{9fff}').contains(&c)
            }) > 0.3
        }
        "en" => share(|c| c.is_ascii()) > 0.9,
        _ => false,
    }
}

/// 集めた記事の見出し・要約を、`code` の言語へ翻訳して併記する。すでにその言語のものは翻訳しない。
/// `@@n@@` 行形式で約2,000文字ごとに分け、4つまで同時に翻訳する。翻訳できた件数を返す。
async fn translate_items(
    http: &reqwest::Client,
    base: &str,
    items: &mut [Item],
    code: &str,
    native: &str,
) -> Result<usize> {
    const CHUNK_CHARS: usize = 2_000;
    let mut src: Vec<(usize, String)> = Vec::new();
    for (i, it) in items.iter().enumerate() {
        if it.source == "news" || looks_like(code, &format!("{} {}", it.title, it.snippet)) {
            continue;
        }
        src.push((i * 2 + 1, it.title.chars().take(200).collect()));
        if !it.snippet.is_empty() {
            src.push((i * 2 + 2, it.snippet.chars().take(300).collect()));
        }
    }
    if src.is_empty() {
        return Ok(0);
    }
    let mut chunks: Vec<Vec<(usize, String)>> = vec![Vec::new()];
    let mut size = 0;
    for item in src {
        let len = item.1.chars().count();
        if size + len > CHUNK_CHARS && !chunks.last().unwrap().is_empty() {
            chunks.push(Vec::new());
            size = 0;
        }
        size += len;
        chunks.last_mut().unwrap().push(item);
    }
    let mut results: Vec<Result<HashMap<usize, String>>> = Vec::new();
    for group in chunks.chunks(4) {
        let futs: Vec<_> = group
            .iter()
            .map(|c| translate_chunk(http, base, c, code, native))
            .collect();
        results.extend(futures::future::join_all(futs).await);
    }
    let mut done: HashMap<usize, String> = HashMap::new();
    for r in results {
        done.extend(r?);
    }
    let mut n = 0;
    for (i, it) in items.iter_mut().enumerate() {
        if let Some(t) = done.get(&(i * 2 + 1)) {
            it.title_tr = Some(t.clone());
            n += 1;
        }
        it.snippet_tr = done.get(&(i * 2 + 2)).cloned();
    }
    Ok(n)
}
fn parse_analysis(text: &str, n_items: usize) -> Option<Analysis> {
    let j = extract_json(text)?;
    let mut a: Analysis = serde_json::from_value(j).ok()?;
    if a.summary.trim().is_empty() {
        return None;
    }
    // 存在しない番号の根拠は捨てる(AI の取り違え・でっち上げ対策)
    let valid = |v: &mut Vec<usize>| {
        v.retain(|&n| (1..=n_items).contains(&n));
        v.dedup();
    };
    for f in a
        .trends
        .iter_mut()
        .chain(a.opportunities.iter_mut())
        .chain(a.risks.iter_mut())
    {
        valid(&mut f.evidence);
    }
    for p in a.proposals.iter_mut() {
        valid(&mut p.evidence);
        let pr = p.priority.to_ascii_lowercase();
        p.priority = if ["high", "medium", "low"].contains(&pr.as_str()) {
            pr
        } else {
            "medium".into()
        };
    }
    for s in a.sentiment.iter_mut() {
        s.score = if s.score.is_finite() {
            s.score.clamp(-2.0, 2.0)
        } else {
            0.0
        };
    }
    Some(a)
}

async fn analyze(
    http: &reqwest::Client,
    base: &str,
    prompt: &str,
    n_items: usize,
) -> Result<(Analysis, Option<String>)> {
    let (text, provider) = complete(http, base, prompt).await?;
    if let Some(a) = parse_analysis(&text, n_items) {
        return Ok((a, provider));
    }
    // 形式が崩れていたら1回だけ、JSON だけを返すよう念を押して再試行する
    let retry = format!("{prompt}\n\nIMPORTANT: Your previous answer was not valid JSON. Output ONLY the JSON object.");
    let (text, provider) = complete(http, base, &retry).await?;
    parse_analysis(&text, n_items)
        .map(|a| (a, provider))
        .ok_or_else(|| {
            anyhow!("AI の分析結果を読み取れませんでした(JSON 形式ではありませんでした)")
        })
}

/// 翻訳が必要な文章を、決まった順で取り出す(構造・数値・根拠は含めない)。
fn texts(a: &Analysis) -> Vec<String> {
    let mut v = vec![a.summary.clone()];
    for f in a.trends.iter().chain(&a.opportunities).chain(&a.risks) {
        v.push(f.title.clone());
        v.push(f.detail.clone());
    }
    for p in &a.proposals {
        v.push(p.action.clone());
        v.push(p.why.clone());
    }
    v.extend(a.sentiment.iter().map(|s| s.reason.clone()));
    v
}

/// [`texts`] と同じ順で文章を差し替える。
fn with_texts(a: &Analysis, t: Vec<String>) -> Analysis {
    let mut out = a.clone();
    let mut it = t.into_iter();
    let mut next = || it.next().unwrap_or_default();
    out.summary = next();
    for f in out
        .trends
        .iter_mut()
        .chain(out.opportunities.iter_mut())
        .chain(out.risks.iter_mut())
    {
        f.title = next();
        f.detail = next();
    }
    for p in out.proposals.iter_mut() {
        p.action = next();
        p.why = next();
    }
    for s in out.sentiment.iter_mut() {
        s.reason = next();
    }
    out
}

/// `@@n@@ 訳文` 形式の行を読み取る(n は 1 始まり)。
fn parse_marked_lines(text: &str) -> HashMap<usize, String> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let line = line.trim().trim_start_matches(['-', '*', ' ']);
        let Some(rest) = line.strip_prefix("@@") else {
            continue;
        };
        let Some((num, body)) = rest.split_once("@@") else {
            continue;
        };
        if let Ok(n) = num.trim().parse::<usize>() {
            let body = body.trim();
            if !body.is_empty() {
                out.insert(n, body.to_string());
            }
        }
    }
    out
}

/// 文章をまとめて翻訳する。`@@n@@ 訳文` の1行形式で返させる(JSON はやめた。レポートの文章に
/// 引用符などが混ざると AI が JSON のエスケープを誤り、読めなくなるため。2026-09-24、ヒンディー語で実例)。
/// 返ってこなかった番号だけ、もう1回依頼する。
async fn translate_chunk(
    http: &reqwest::Client,
    base: &str,
    chunk: &[(usize, String)],
    code: &str,
    native: &str,
) -> Result<HashMap<usize, String>> {
    let mut done: HashMap<usize, String> = HashMap::new();
    for _ in 0..2 {
        let todo: Vec<&(usize, String)> = chunk
            .iter()
            .filter(|(n, _)| !done.contains_key(n))
            .collect();
        if todo.is_empty() {
            break;
        }
        let body: String = todo
            .iter()
            .map(|(n, s)| format!("@@{n}@@ {}\n", s.replace('\n', " ")))
            .collect();
        let prompt = format!(
            "Translate the text after each @@n@@ marker into {native} (language code {code}). Keep [n] citation markers, \
             numbers and proper nouns unchanged. Output exactly one line per item, in the form `@@n@@ translation`, \
             keeping the same numbers, and nothing else.\n\n{body}"
        );
        let (text, _) = complete(http, base, &prompt).await?;
        let got = parse_marked_lines(&text);
        for (n, _) in &todo {
            if let Some(v) = got.get(n) {
                done.insert(*n, v.clone());
            }
        }
    }
    Ok(done)
}

/// 分析結果を別の言語へ翻訳する。AI には文章だけを渡し、構造・数値・根拠・優先度は
/// サーバー側で元の結果から組み立てる(言語ごとに内容が食い違わない)。
/// 長い文章は約 2,000 文字ずつに分けて並行で翻訳する。
async fn translate_analysis(
    http: &reqwest::Client,
    base: &str,
    a: &Analysis,
    code: &str,
    native: &str,
) -> Result<(Analysis, Option<String>)> {
    const CHUNK_CHARS: usize = 2_000;
    let src: Vec<(usize, String)> = texts(a)
        .into_iter()
        .enumerate()
        .map(|(i, s)| (i + 1, s))
        .collect();
    let mut chunks: Vec<Vec<(usize, String)>> = vec![Vec::new()];
    let mut size = 0;
    for item in src.iter().cloned() {
        let len = item.1.chars().count();
        if size + len > CHUNK_CHARS && !chunks.last().unwrap().is_empty() {
            chunks.push(Vec::new());
            size = 0;
        }
        size += len;
        chunks.last_mut().unwrap().push(item);
    }
    let results = futures::future::join_all(
        chunks
            .iter()
            .map(|c| translate_chunk(http, base, c, code, native)),
    )
    .await;
    let mut all: HashMap<usize, String> = HashMap::new();
    for r in results {
        all.extend(r?);
    }
    let missing = src
        .iter()
        .filter(|(n, s)| !s.trim().is_empty() && !all.contains_key(n))
        .count();
    if missing > 0 {
        bail!("{native} への翻訳で {missing} 件の文章が返ってきませんでした");
    }
    let translated = src
        .iter()
        .map(|(n, s)| all.get(n).cloned().unwrap_or_else(|| s.clone()))
        .collect();
    Ok((with_texts(a, translated), None))
}
pub async fn run(
    http: &reqwest::Client,
    base: &str,
    regions: &RegionData,
    opt: Options,
) -> Result<Outcome> {
    let start = Instant::now();
    let theme = opt.theme.trim().to_string();
    if theme.chars().count() > 200 {
        bail!("テーマは200文字までにしてください");
    }
    let topics: Vec<&'static Topic> = opt
        .topics
        .iter()
        .map(|id| {
            places::topic(id).ok_or_else(|| anyhow!("知りたい情報の指定が正しくありません: {id}"))
        })
        .collect::<Result<Vec<_>>>()?;
    if theme.is_empty() && topics.is_empty() {
        bail!("テーマを入力するか、知りたい情報にチェックを入れてください");
    }
    if opt.places.is_empty() || opt.places.len() > MAX_PLACES {
        bail!("調べる場所は1〜{MAX_PLACES}か所選んでください");
    }
    let targets = opt
        .places
        .iter()
        .map(|p| places::resolve(p, regions))
        .collect::<Result<Vec<_>>>()?;
    if opt.languages.is_empty() || opt.languages.len() > crate::explain::MAX_LANGUAGES {
        bail!(
            "説明の言語は1〜{}個選んでください",
            crate::explain::MAX_LANGUAGES
        );
    }
    let langs = opt
        .languages
        .iter()
        .map(|l| languages::find(l).ok_or_else(|| anyhow!("対応していない言語コードです: {l}")))
        .collect::<Result<Vec<_>>>()?;
    if opt.search_languages.len() > MAX_EXTRA_LANGS {
        bail!("追加の検索言語は{MAX_EXTRA_LANGS}個までです");
    }
    let mut extra_langs = Vec::new();
    for l in &opt.search_languages {
        let f = languages::find(l).ok_or_else(|| anyhow!("対応していない検索言語です: {l}"))?;
        if !extra_langs.iter().any(|e: &(&str, &str, &str)| e.0 == f.0) {
            extra_langs.push(f);
        }
    }
    let per = opt.per_country.clamp(3, MAX_PER_COUNTRY);
    let mut warnings = Vec::new();

    // 1. 検索の組み立て: 場所 × 検索言語 × (テーマ + 知りたい情報)。回数が多すぎないように数える。
    // 検索する言語は、その国の言語(現地語)と、追加で選んだ言語。
    let search_langs_of = |t: &Target| -> Vec<(&'static str, &'static str, String)> {
        let own = languages::find(t.lang).expect("Target.lang は一覧にある言語");
        let mut v = vec![(own.0, own.1, t.hl.clone())];
        for &(code, ja, _) in &extra_langs {
            if code != own.0 {
                v.push((code, ja, hl_for(code)));
            }
        }
        v
    };
    let planned: usize = targets
        .iter()
        .map(|t| search_langs_of(t).len() * (usize::from(!theme.is_empty()) + topics.len()))
        .sum();
    if planned > MAX_SEARCHES {
        bail!(
            "検索の回数が多すぎます({planned}回、上限{MAX_SEARCHES}回)。場所・知りたい情報・追加の検索言語のどれかを減らしてください"
        );
    }

    // 2. テーマと知りたい情報の翻訳(現地語ごと)。日本語・英語の知りたい情報は固定の検索語を使う。
    // GitHub / YouTube は英語で検索するため、英語は常に含める
    let mut need: Vec<&str> = targets
        .iter()
        .flat_map(|t| search_langs_of(t).into_iter().map(|x| x.0))
        .chain(["en"])
        .collect();
    need.sort_unstable();
    need.dedup();
    let translated = if theme.is_empty() {
        HashMap::new()
    } else {
        match translate_theme(http, base, &theme, &need).await {
            Ok(m) => m,
            Err(e) => {
                warnings.push(format!(
                    "テーマの翻訳に失敗したため、入力したテーマのまま検索しました({e:#})"
                ));
                HashMap::new()
            }
        }
    };
    let topic_langs: Vec<&str> = need
        .iter()
        .copied()
        .filter(|l| *l != "ja" && *l != "en")
        .collect();
    let topic_tr = if topics.is_empty() || topic_langs.is_empty() {
        HashMap::new()
    } else {
        match translate_topics(http, base, &topics, &topic_langs).await {
            Ok(m) => m,
            Err(e) => {
                warnings.push(format!(
                    "知りたい情報の翻訳に失敗したため、英語の検索語で検索しました({e:#})"
                ));
                HashMap::new()
            }
        }
    };

    // 3. 検索の一覧を作る
    let whole_by_lang = |lang: &str| {
        targets
            .iter()
            .filter(|u| u.whole_country && u.lang == lang)
            .count()
            > 1
    };
    let mut jobs: Vec<Job> = Vec::new();
    for (ti, t) in targets.iter().enumerate() {
        for (lang, lang_ja, hl) in search_langs_of(t) {
            let is_own = lang == t.lang;
            if !theme.is_empty() {
                let q = translated
                    .get(lang)
                    .cloned()
                    .unwrap_or_else(|| theme.clone());
                // 同じ言語の国全体を複数選んだ場合は国名を付ける。付けないと検索語が同じになり、検索エンジンが
                // ほぼ同じ結果を返して重複排除で片方の国の情報が消える(2026-09-24、アメリカとインドで実例)。
                let query = if !t.whole_country {
                    format!("{q} {}", t.place_text)
                } else if (is_own && whole_by_lang(lang)) || (!is_own && targets.len() > 1) {
                    format!("{q} {}", t.country_en)
                } else {
                    q.clone()
                };
                jobs.push(Job {
                    target: ti,
                    topic: None,
                    lang_ja,
                    gl: t.gl.clone(),
                    hl: hl.clone(),
                    query,
                    key: q,
                    is_own,
                    kind: Kind::Web,
                });
            }
            for tp in &topics {
                let phrase = places::phrase_for(tp, lang, &topic_tr);
                jobs.push(Job {
                    target: ti,
                    topic: Some(tp),
                    lang_ja,
                    gl: t.gl.clone(),
                    hl: hl.clone(),
                    query: format!("{} {phrase}", t.place_text),
                    key: phrase,
                    is_own,
                    kind: Kind::Web,
                });
            }
        }
        if opt.include_news && t.whole_country {
            jobs.push(Job {
                target: ti,
                topic: None,
                lang_ja: "",
                gl: String::new(),
                hl: String::new(),
                query: t.country_en.clone(),
                key: String::new(),
                is_own: true,
                kind: Kind::News,
            });
        }
    }
    let queries: Vec<(String, String)> = jobs
        .iter()
        .filter(|j| matches!(j.kind, Kind::Web))
        .map(|j| {
            let t = &targets[j.target];
            let mut label = t.label.clone();
            if let Some(tp) = j.topic {
                label.push_str(&format!(" · {}", tp.label));
            }
            if !j.is_own {
                label.push_str(&format!(" · {}で検索", j.lang_ja));
            }
            (label, j.query.clone())
        })
        .collect();

    // 4. 収集(最大6件を同時に。結果は元の順に並べ直す)
    let targets_ref = &targets;
    let mut results: Vec<(usize, Vec<Item>, Option<String>)> = futures::stream::iter(jobs.into_iter().enumerate())
        .map(|(idx, job)| async move {
            let t = &targets_ref[job.target];
            match job.kind {
                Kind::Web => {
                    let body = serde_json::json!({ "source": "google", "query": job.query, "max_results": per, "gl": job.gl, "hl": job.hl });
                    match search_raw(http, base, body).await {
                        Ok(rs) => {
                            let items = rs
                                .iter()
                                .map(|r| Item {
                                    country: t.label.clone(),
                                    topic: job.topic.map_or("テーマ".to_string(), |x| x.label.to_string()),
                                    source: "web",
                                    query: job.query.clone(),
                                    key: job.key.clone(),
                                    title_tr: None,
                                    snippet_tr: None,
                                    title: s(r, &["title"]),
                                    snippet: s(r, &["snippet"]),
                                    url: s(r, &["link", "url"]),
                                })
                                .collect();
                            (idx, items, None)
                        }
                        Err(e) => (idx, vec![], Some(format!("{} の Web 検索に失敗: {e:#}", t.label))),
                    }
                }
                Kind::News => match country_news(http, base, &t.country_en).await {
                    Ok(rs) => {
                        let items = rs
                            .iter()
                            .take(per as usize)
                            .map(|r| Item {
                                country: t.label.clone(),
                                topic: "国の一般情勢".into(),
                                source: "news",
                                query: format!("{} headlines", t.country_en),
                                key: String::new(),
                                title_tr: None,
                                snippet_tr: None,
                                title: s(r, &["title"]),
                                snippet: s(r, &["snippet"]),
                                url: s(r, &["link"]),
                            })
                            .collect();
                        (idx, items, None)
                    }
                    Err(e) => (idx, vec![], Some(format!("{}のニュース取得に失敗: {e:#}", t.label))),
                },
            }
        })
        .buffer_unordered(6)
        .collect()
        .await;
    results.sort_by_key(|r| r.0);
    let mut items: Vec<Item> = Vec::new();
    for (_, out, warn) in results {
        items.extend(out);
        warnings.extend(warn);
    }
    let english = translated
        .get("en")
        .cloned()
        .unwrap_or_else(|| theme.clone());
    for (flag, src) in [
        (opt.include_github, "github"),
        (opt.include_youtube, "youtube"),
    ] {
        if !flag {
            continue;
        }
        if theme.is_empty() {
            warnings.push(format!("テーマが空のため、{src} は検索しませんでした"));
            continue;
        }
        match search_raw(
            http,
            base,
            serde_json::json!({ "source": src, "query": english, "max_results": per }),
        )
        .await
        {
            Ok(rs) => items.extend(rs.iter().map(|r| Item {
                country: "Global".into(),
                topic: "テーマ".into(),
                source: if src == "github" { "github" } else { "youtube" },
                query: english.clone(),
                key: english.clone(),
                title_tr: None,
                snippet_tr: None,
                title: s(r, &["full_name", "title"]),
                snippet: s(r, &["description", "channel_title", "snippet"]),
                url: s(r, &["url", "link"]),
            })),
            Err(e) => warnings.push(format!("{src} の検索に失敗: {e:#}")),
        }
    }
    // 同じ URL は1件にまとめる
    let mut seen = std::collections::HashSet::new();
    items.retain(|it| !it.title.is_empty() && (it.url.is_empty() || seen.insert(it.url.clone())));
    // テーマ・知りたい情報と無関係な Web 検索結果を除く(その検索の主要な語を含むものだけ残す。
    // ニュースは国の一般情勢として別扱いなので対象外)
    let before = items.len();
    items.retain(|it| it.source != "web" || is_relevant(it, &topic_tokens(&it.key)));
    if items.len() < before {
        warnings.push(format!(
            "検索した内容の主要な語を含まない Web 検索結果 {} 件を除外しました",
            before - items.len()
        ));
    }
    // 番号はテーマの情報を先に、国別ニュース(一般情勢)を後にする
    items.sort_by_key(|it| it.source == "news");
    if items.is_empty() {
        bail!("情報を1件も集められませんでした。{}", warnings.join(" / "));
    }

    // 記事の見出し・要約の翻訳版(選んだ場合)。翻訳先は先頭のレポート言語。
    if opt.translate_items {
        let (tcode, _, tnative) = langs[0];
        match translate_items(http, base, &mut items, tcode, tnative).await {
            Ok(0) => warnings.push("翻訳が必要な記事はありませんでした(すべて同じ言語です)".into()),
            Ok(_) => {}
            Err(e) => warnings.push(format!("記事の翻訳に失敗しました({e:#})")),
        }
    }

    // 5. データセット化
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .to_string();
    let host = |u: &str| {
        reqwest::Url::parse(u).ok().and_then(|u| {
            u.host_str()
                .map(|h| h.trim_start_matches("www.").to_string())
        })
    };
    let rows = items
        .iter()
        .enumerate()
        .map(|(i, it)| {
            vec![
                Some((i + 1).to_string()),
                Some(it.country.clone()),
                Some(it.topic.clone()),
                Some(it.source.to_string()),
                Some(it.title.clone()),
                Some(it.snippet.clone()).filter(|s| !s.is_empty()),
                it.title_tr.clone(),
                it.snippet_tr.clone(),
                Some(it.url.clone()).filter(|s| !s.is_empty()),
                host(&it.url),
                Some(it.query.clone()),
                Some(now.clone()),
            ]
        })
        .collect();
    let df = rrd_core::csv::from_records(
        [
            "no",
            "place",
            "topic",
            "source",
            "title",
            "snippet",
            "title_translated",
            "snippet_translated",
            "url",
            "host",
            "query",
            "retrieved_at_unix",
        ]
        .map(String::from)
        .to_vec(),
        rows,
    )
    .map_err(|e| anyhow!("{e}"))?;

    // 6. 分析(1つ目の言語)→ 7. 他の言語へ翻訳
    let country_names: Vec<String> = targets.iter().map(|t| t.label.clone()).collect();
    let display_theme = if theme.is_empty() {
        topics
            .iter()
            .map(|t| t.label)
            .collect::<Vec<_>>()
            .join("、")
    } else {
        theme.clone()
    };
    let extra = extra_context(&topics);
    let b = brief(&display_theme, &items);
    let (code0, ja0, native0) = langs[0];
    let (first, prov0) = analyze(
        http,
        base,
        &analysis_prompt(&b, code0, native0, &country_names, &extra),
        items.len(),
    )
    .await?;
    let mut reports = vec![Report {
        lang: code0.into(),
        language_name: ja0.into(),
        analysis: first.clone(),
        provider: prov0,
    }];
    let n_items = items.len();
    let rest = langs[1..].iter().map(|&(code, ja, native)| {
        let first = &first;
        let (b, names, extra) = (&b, &country_names, &extra);
        async move {
            // 翻訳(内容を揃える)を優先し、失敗したらその言語で直接分析する
            let r = match translate_analysis(http, base, first, code, native).await {
                Ok(v) => Ok((v, None)),
                Err(e) => analyze(
                    http,
                    base,
                    &analysis_prompt(b, code, native, names, extra),
                    n_items,
                )
                .await
                .map(|v| {
                    (
                        v,
                        Some(format!(
                            "{ja}版は翻訳に失敗したため、{ja}で直接分析しました({e:#})"
                        )),
                    )
                }),
            };
            (code, ja, r)
        }
    });
    for (code, ja, r) in futures::future::join_all(rest).await {
        match r {
            Ok(((a, p), note)) => {
                warnings.extend(note);
                reports.push(Report {
                    lang: code.into(),
                    language_name: ja.into(),
                    analysis: a,
                    provider: p,
                });
            }
            Err(e) => warnings.push(format!("{ja}版の作成に失敗: {e:#}")),
        }
    }
    Ok(Outcome {
        items,
        df,
        reports,
        warnings,
        queries,
        millis: start.elapsed().as_secs_f64() * 1000.0,
    })
}

/// 国別・情報源別の件数(可視化用)。
pub fn counts(items: &[Item]) -> (BTreeMap<String, usize>, BTreeMap<String, usize>) {
    let mut by_country = BTreeMap::new();
    let mut by_source = BTreeMap::new();
    for it in items {
        *by_country.entry(it.country.clone()).or_insert(0) += 1;
        *by_source.entry(it.source.to_string()).or_insert(0) += 1;
    }
    (by_country, by_source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_json_from_fenced_answer() {
        let j = extract_json("Sure!\n```json\n{\"a\": 1, \"b\": {\"c\": 2}}\n```\nthanks").unwrap();
        assert_eq!(j["b"]["c"], 2);
        assert!(extract_json("no json here").is_none());
    }

    #[test]
    fn parse_analysis_drops_invalid_evidence_and_clamps() {
        let text = r#"{"summary":"s","trends":[{"title":"t","detail":"d","countries":["Japan"],"evidence":[1,2,99,0,2]}],
            "proposals":[{"action":"a","why":"w","priority":"URGENT","evidence":[3]}],
            "sentiment":[{"country":"Japan","score":7,"reason":"r"}]}"#;
        let a = parse_analysis(text, 3).unwrap();
        assert_eq!(a.trends[0].evidence, vec![1, 2]);
        assert_eq!(a.proposals[0].priority, "medium");
        assert_eq!(a.sentiment[0].score, 2.0);
        assert!(a.opportunities.is_empty());
        assert!(parse_analysis(r#"{"summary":""}"#, 3).is_none());
    }

    #[test]
    fn brief_stays_within_limit_and_keeps_numbers() {
        let items: Vec<Item> = (0..200)
            .map(|i| Item {
                country: "Japan".into(),
                topic: String::new(),
                key: String::new(),
                source: "web",
                query: "q".into(),
                title_tr: None,
                snippet_tr: None,
                title: format!("title {i}"),
                snippet: "x".repeat(400),
                url: format!("https://e.com/{i}"),
            })
            .collect();
        let b = brief("theme", &items);
        assert!(
            b.chars().count() <= MAX_BRIEF_CHARS + 200,
            "{}",
            b.chars().count()
        );
        assert!(b.contains("[1] Japan") && b.contains("[200] Japan"));
    }

    #[test]
    fn relevance_filter_drops_generic_matches() {
        let tokens: Vec<String> = ["抹茶 海外需要", "matcha global demand"]
            .into_iter()
            .flat_map(topic_tokens)
            .collect();
        assert!(tokens.contains(&"抹茶".to_string()) && tokens.contains(&"matcha".to_string()));
        assert!(!tokens.contains(&"demand".to_string()));
        let item = |t: &str| Item {
            country: "Japan".into(),
            topic: String::new(),
            key: String::new(),
            source: "web",
            query: String::new(),
            title_tr: None,
            snippet_tr: None,
            title: t.into(),
            snippet: String::new(),
            url: String::new(),
        };
        assert!(is_relevant(&item("抹茶ブームの裏側"), &tokens));
        assert!(is_relevant(&item("The MATCHA boom"), &tokens));
        assert!(!is_relevant(&item("通信機器中期需要予測"), &tokens));
        assert!(is_relevant(&item("anything"), &[]));
    }

    #[test]
    fn brief_separates_general_headlines() {
        let mk = |src: &'static str, t: &str| Item {
            country: "Japan".into(),
            topic: String::new(),
            key: String::new(),
            source: src,
            query: String::new(),
            title_tr: None,
            snippet_tr: None,
            title: t.into(),
            snippet: String::new(),
            url: String::new(),
        };
        let b = brief("x", &[mk("web", "A"), mk("news", "B")]);
        let (topic, ctx) = b.split_once("General national headlines").unwrap();
        assert!(
            topic.contains("[1] Japan |  | web | A") && ctx.contains("[2] Japan |  | news | B")
        );
    }

    #[test]
    fn translation_roundtrip_keeps_structure() {
        let a = Analysis {
            summary: "s".into(),
            trends: vec![Finding {
                title: "t".into(),
                detail: "d".into(),
                countries: vec!["Japan".into()],
                evidence: vec![1],
            }],
            proposals: vec![Proposal {
                action: "a".into(),
                why: "w".into(),
                priority: "high".into(),
                evidence: vec![2],
            }],
            sentiment: vec![Sentiment {
                country: "Japan".into(),
                score: 1.5,
                reason: "r".into(),
            }],
            ..Default::default()
        };
        let src = texts(&a);
        assert_eq!(src, ["s", "t", "d", "a", "w", "r"]);
        let tr = with_texts(&a, src.iter().map(|s| format!("<{s}>")).collect());
        assert_eq!(tr.summary, "<s>");
        assert_eq!(tr.trends[0].detail, "<d>");
        assert_eq!(tr.trends[0].evidence, vec![1]);
        assert_eq!(tr.proposals[0].priority, "high");
        assert_eq!(tr.sentiment[0].score, 1.5);
        assert_eq!(tr.sentiment[0].reason, "<r>");
        let m =
            parse_marked_lines("ok\n@@1@@ 一つ目 \"引用\" [2]\n- @@2@@ second\n@@x@@ bad\n@@3@@\n");
        assert_eq!(m.get(&1).map(String::as_str), Some("一つ目 \"引用\" [2]"));
        assert_eq!(m.get(&2).map(String::as_str), Some("second"));
        assert_eq!(m.len(), 2);
    }

    #[test]
    fn countries_have_known_languages() {
        for c in COUNTRIES {
            assert!(
                languages::find(c.4).is_some(),
                "{} の言語 {} が一覧に無い",
                c.0,
                c.4
            );
        }
    }
}
