//! GitHub REST API の最小限の呼び出し。
//!
//! トークンは環境変数 `RRD_GITHUB_TOKEN`(無ければ `GITHUB_TOKEN`)から読む。git 本体の push/pull
//! 認証(credential store 等)とは別に、この API 呼び出し専用でトークンを使う(容量確認・新規
//! リポジトリの自動作成にしか使わない。読み書き自体は [`crate::archive`] が git で行う)。

use anyhow::{bail, Context, Result};
use serde_json::json;

fn token() -> Option<String> {
    std::env::var("RRD_GITHUB_TOKEN")
        .ok()
        .or_else(|| std::env::var("GITHUB_TOKEN").ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent("realdata.pro-vault")
        .build()
        .context("HTTP クライアントを作れません")
}

/// `owner/repo` 形式・clone URL のどちらでも受け付け、`(owner, repo)` を返す。
pub fn owner_repo(repo_url: &str) -> Option<(String, String)> {
    let s = repo_url
        .trim()
        .trim_end_matches(".git")
        .trim_end_matches('/');
    let s = s
        .strip_prefix("git@github.com:")
        .or_else(|| s.strip_prefix("https://github.com/"))
        .or_else(|| s.strip_prefix("http://github.com/"))
        .unwrap_or(s);
    let (owner, name) = s.rsplit_once('/')?;
    if name.is_empty() || owner.is_empty() {
        return None;
    }
    Some((owner.to_string(), name.to_string()))
}

/// リポジトリの現在のディスク使用量(バイト。GitHub API はキロバイト単位で返す)。
pub async fn repo_size_bytes(repo_url: &str) -> Result<u64> {
    let Some(tok) = token() else {
        bail!("RRD_GITHUB_TOKEN(または GITHUB_TOKEN)が未設定です");
    };
    let (owner, name) = owner_repo(repo_url).context("リポジトリの owner/name を取り出せません")?;
    let resp = client()?
        .get(format!("https://api.github.com/repos/{owner}/{name}"))
        .bearer_auth(tok)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("GitHub API に接続できません")?;
    if !resp.status().is_success() {
        bail!("GitHub API が {} を返しました", resp.status());
    }
    let json: serde_json::Value = resp.json().await.context("GitHub API の応答を読めません")?;
    let kb = json
        .get("size")
        .and_then(|v| v.as_u64())
        .context("応答に size フィールドがありません")?;
    Ok(kb.saturating_mul(1024))
}

/// 新しい非公開リポジトリを作る(既に同名があれば、それをそのまま使う)。clone URL(https)を返す。
pub async fn create_private_repo(owner: &str, name: &str) -> Result<String> {
    let Some(tok) = token() else {
        bail!("RRD_GITHUB_TOKEN(または GITHUB_TOKEN)が未設定です");
    };
    let c = client()?;
    let body = json!({
        "name": name,
        "private": true,
        "description": "realdata.pro のデータ保管庫(容量上限に近づいたため自動作成したシャード)",
        "auto_init": true,
    });
    // 組織(org)向けをまず試し、org でなければ個人アカウント向けにする
    let org_resp = c
        .post(format!("https://api.github.com/orgs/{owner}/repos"))
        .bearer_auth(&tok)
        .header("Accept", "application/vnd.github+json")
        .json(&body)
        .send()
        .await
        .context("GitHub API に接続できません")?;
    let resp = if org_resp.status() == reqwest::StatusCode::NOT_FOUND {
        c.post("https://api.github.com/user/repos")
            .bearer_auth(&tok)
            .header("Accept", "application/vnd.github+json")
            .json(&body)
            .send()
            .await
            .context("GitHub API に接続できません")?
    } else {
        org_resp
    };
    // 既に存在する(422 name already exists)場合は、そのリポジトリを使う
    if resp.status() == reqwest::StatusCode::UNPROCESSABLE_ENTITY {
        return Ok(format!("https://github.com/{owner}/{name}.git"));
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!(
            "リポジトリを作成できません({status}): {}",
            text.chars().take(500).collect::<String>()
        );
    }
    let json: serde_json::Value = resp.json().await.context("GitHub API の応答を読めません")?;
    let clone_url = json
        .get("clone_url")
        .and_then(|v| v.as_str())
        .unwrap_or(&format!("https://github.com/{owner}/{name}.git"))
        .to_string();
    Ok(clone_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_owner_repo_from_various_forms() {
        assert_eq!(
            owner_repo("https://github.com/aon-co-jp/realdata-archive.git"),
            Some(("aon-co-jp".into(), "realdata-archive".into()))
        );
        assert_eq!(
            owner_repo("https://github.com/aon-co-jp/realdata-archive"),
            Some(("aon-co-jp".into(), "realdata-archive".into()))
        );
        assert_eq!(
            owner_repo("git@github.com:aon-co-jp/realdata-archive.git"),
            Some(("aon-co-jp".into(), "realdata-archive".into()))
        );
        assert_eq!(
            owner_repo("aon-co-jp/realdata-archive"),
            Some(("aon-co-jp".into(), "realdata-archive".into()))
        );
        assert_eq!(owner_repo("not-a-repo"), None);
        assert_eq!(owner_repo(""), None);
    }
}
