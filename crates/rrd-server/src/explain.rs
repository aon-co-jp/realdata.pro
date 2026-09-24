//! 分析結果の説明(aruaru-llm)。日本語・英語と、利用者が選んだ言語で説明する。
//!
//! AI に渡すのは要約統計と、利用者が画面で見ている分析結果(表や回帰式)だけにする。
//! 生データの行は渡さない(利用者が明示的に「サンプル行も渡す」を選んだ場合を除く)。
//! 外部の AI 事業者へ送られる可能性があるため、呼び出し側で同意を必須にしている。

use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use rrd_core::{DataFrame, Value};
use serde_json::Value as Json;

use crate::languages;

/// 1回の説明で指定できる言語の数(日本語・英語・選択した言語)。
pub const MAX_LANGUAGES: usize = 3;
const MAX_CONTEXT_CHARS: usize = 8_000;
const SAMPLE_ROWS: usize = 10;

pub struct Explanation {
    pub lang: String,
    pub language_name: String,
    pub text: String,
    pub provider: Option<String>,
}

fn table_text(df: &DataFrame, max_rows: usize) -> String {
    let mut s = df
        .columns()
        .iter()
        .map(|c| c.name.as_str())
        .collect::<Vec<_>>()
        .join("\t");
    s.push('\n');
    for i in 0..df.height().min(max_rows) {
        let row: Vec<String> = df
            .columns()
            .iter()
            .map(|c| match c.get(i) {
                Value::Null => "null".into(),
                Value::Float(f) => format!("{f:.6}")
                    .trim_end_matches('0')
                    .trim_end_matches('.')
                    .to_string(),
                v => v.to_string(),
            })
            .collect();
        s.push_str(&row.join("\t"));
        s.push('\n');
    }
    s
}

/// 言語に依存しない、AI へ渡す分析資料。
pub fn build_brief(
    name: &str,
    df: &DataFrame,
    analysis: Option<&str>,
    include_sample: bool,
) -> Result<String> {
    let mut brief = format!(
        "Dataset \"{name}\": {} rows x {} columns.\nColumn types: {}\n\nSummary statistics (TSV):\n{}",
        df.height(),
        df.width(),
        df.columns().iter().map(|c| format!("{}={}", c.name, c.dtype())).collect::<Vec<_>>().join(", "),
        table_text(&df.describe(), usize::MAX),
    );
    if include_sample {
        brief.push_str(&format!(
            "\nFirst {} rows (TSV):\n{}",
            SAMPLE_ROWS.min(df.height()),
            table_text(df, SAMPLE_ROWS)
        ));
    }
    if let Some(a) = analysis.map(str::trim).filter(|a| !a.is_empty()) {
        if a.chars().count() > MAX_CONTEXT_CHARS {
            bail!("分析結果の文字数が多すぎます(上限 {MAX_CONTEXT_CHARS} 文字)");
        }
        brief.push_str(&format!("\nAnalysis result the user is looking at:\n{a}\n"));
    }
    Ok(brief)
}

fn prompt_for(brief: &str, code: &str, native: &str) -> String {
    format!(
        "You are a data analyst helping business owners with no statistics background.\n\
         Explain the following analysis in {native} (language code: {code}). Write ONLY in that language.\n\
         Structure: (1) what the data shows in plain words, (2) 3 key findings with the actual numbers,\n\
         (3) cautions (missing values, small sample, correlation is not causation), (4) 2-3 practical next steps.\n\
         Do not invent numbers that are not in the material. Do not assume units or currency\n\
         (e.g. yen, dollars) unless the material states them; quote numbers as they are. Keep it concise.\n\n{brief}"
    )
}

/// aruaru-llm(無料 AI を優先順・ハイブリッドで使う `complete-priority`)に1回問い合わせ、
/// (回答, AI 名)を返す。
pub async fn complete(
    http: &reqwest::Client,
    llm_base: &str,
    prompt: &str,
) -> Result<(String, Option<String>)> {
    let url = format!(
        "{}/v1/chat-providers/complete-priority",
        llm_base.trim_end_matches('/')
    );
    let resp = http
        .post(&url)
        .json(&serde_json::json!({ "prompt": prompt }))
        .timeout(Duration::from_secs(150))
        .send()
        .await
        .with_context(|| format!("aruaru-llm({llm_base})に接続できません"))?;
    let status = resp.status();
    let j: Json = resp.json().await.context("aruaru-llm の応答を読めません")?;
    if !status.is_success() {
        bail!(
            "aruaru-llm がエラーを返しました: {}",
            j.get("error").and_then(Json::as_str).unwrap_or("不明")
        );
    }
    let reply = j.get("reply").filter(|r| !r.is_null()).ok_or_else(|| {
        if j.get("all_quota_exceeded").and_then(Json::as_bool) == Some(true) {
            anyhow!("利用できる AI がすべて上限に達しています。時間をおいて再度お試しください")
        } else {
            anyhow!("AI から回答を得られませんでした(aruaru-llm に AI のキーが設定されているか確認してください)")
        }
    })?;
    Ok((
        reply
            .get("text")
            .and_then(Json::as_str)
            .unwrap_or_default()
            .to_string(),
        reply.get("provider").map(|p| {
            p.as_str()
                .map(str::to_string)
                .unwrap_or_else(|| p.to_string())
        }),
    ))
}

/// 指定言語それぞれで説明を得る(言語ごとに並行して aruaru-llm を呼ぶ)。
pub async fn explain(
    http: &reqwest::Client,
    llm_base: &str,
    brief: &str,
    langs: &[String],
) -> Result<Vec<Explanation>> {
    if langs.is_empty() || langs.len() > MAX_LANGUAGES {
        bail!("言語は1〜{MAX_LANGUAGES}個指定してください");
    }
    let mut resolved = Vec::new();
    for l in langs {
        let (code, ja, native) =
            languages::find(l).ok_or_else(|| anyhow!("対応していない言語コードです: {l}"))?;
        if !resolved.iter().any(|(c, _, _)| *c == code) {
            resolved.push((code, ja, native));
        }
    }
    let calls = resolved.iter().map(|(code, ja, native)| async move {
        let (text, provider) = complete(http, llm_base, &prompt_for(brief, code, native)).await?;
        Ok(Explanation {
            lang: code.to_string(),
            language_name: ja.to_string(),
            text,
            provider,
        })
    });
    futures::future::join_all(calls).await.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brief_excludes_raw_rows_unless_asked() {
        let df = rrd_core::csv::read_csv_str("secret_name,v\nAlice,1\nBob,2\n").unwrap();
        let b = build_brief("d", &df, Some("v_sum=3"), false).unwrap();
        assert!(!b.contains("Alice") && b.contains("v_sum=3") && b.contains("Summary statistics"));
        assert!(build_brief("d", &df, None, true).unwrap().contains("Alice"));
        assert!(build_brief("d", &df, Some(&"x".repeat(MAX_CONTEXT_CHARS + 1)), false).is_err());
    }

    #[tokio::test]
    async fn rejects_bad_language_requests() {
        let http = reqwest::Client::new();
        assert!(explain(&http, "http://127.0.0.1:1", "b", &[])
            .await
            .is_err());
        assert!(
            explain(&http, "http://127.0.0.1:1", "b", &["xx-unknown".into()])
                .await
                .is_err()
        );
        let four: Vec<String> = ["ja", "en", "fr", "de"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(explain(&http, "http://127.0.0.1:1", "b", &four)
            .await
            .is_err());
    }
}
