//! 複数の GitHub 非公開リポジトリ(シャード)を跨いで「1つの保管庫」に見せる。
//!
//! GitHub は1リポジトリあたり10GBを推奨上限としている(超えると警告・将来の制限の対象になりうる)。
//! `RRD_ARCHIVE_REPO` の1リポジトリだけに保存し続けるとこの上限に近づいていくため、現在書き込み中の
//! シャードのサイズを定期的に確かめ(GitHub API)、しきい値(既定8GB)を超えたら、あらかじめ用意して
//! おいた次のシャードへ**自動的に書き込み先を切り替える**(「引っ越す」)。次のシャードが無ければ、
//! GitHub API で新しいリポジトリを自動作成する。
//!
//! どのファイルがどのシャードにあるかは、索引専用リポジトリ(`RRD_CATALOG_REPO`)の `catalog.json` に
//! 記録する(このファイル自体は小さく、シャード数・パス数に比例するだけで増え続けない)。索引リポジトリが
//! 無い・使えない場合は、これまでどおり単一シャード(`RRD_ARCHIVE_REPO`)のみで動く(自動引っ越しはしない)。
//!
//! 過去のデータは移動しない(コピーコストが大きいため)。引っ越し後も古いシャードは読み込み専用として
//! 残り続け、[`repos_newest_first`] で新しい順に辿れる。

use std::collections::BTreeMap;
use std::sync::OnceLock;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::archive;
use crate::github;

const CATALOG_FILE: &str = "catalog.json";
/// このバイト数を超えたら次のシャードへ引っ越す(GitHub の推奨上限10GBに余裕を持たせる)
const DEFAULT_THRESHOLD_BYTES: u64 = 8 * 1024 * 1024 * 1024;
/// シャードのサイズは GitHub API を毎回叩かず、この間隔でだけ確かめる(書き込みのたびに叩くと遅く・
/// API のレート制限にも近づくため)
const SIZE_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(600);

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct CatalogFile {
    /// シャードの一覧(作成順、末尾が最新の書き込み先)
    shards: Vec<ShardInfo>,
    /// ファイルパス → そのファイルが実際にあるシャードのリポジトリ URL(書き込み時の最新の場所)
    index: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ShardInfo {
    repo: String,
    created_unix: i64,
    /// 上限に達し、書き込みを止めた(読み込みはできる)
    sealed: bool,
}

struct Inner {
    catalog_repo: Option<String>,
    threshold_bytes: u64,
    /// 新しいリポジトリの作成先(組織またはユーザー名)と、名前の先頭部分(例: aon-co-jp / realdata-archive)
    new_repo_owner: Option<String>,
    new_repo_prefix: String,
    catalog: CatalogFile,
    last_size_check: Option<std::time::Instant>,
    dirty: bool,
}

pub struct Vault(Mutex<Inner>);

static VAULT: OnceLock<Vault> = OnceLock::new();

/// 初期化する。`primary_repo` は現行(最初の)シャード。`catalog_repo` を設定した場合だけ、
/// 複数シャードにまたがる自動引っ越し・索引が有効になる(未設定なら単一シャードのまま従来どおり)。
pub async fn connect(
    primary_repo: &str,
    catalog_repo: Option<String>,
    threshold_gb: Option<f64>,
) -> Result<()> {
    let threshold_bytes = threshold_gb
        .filter(|g| *g > 0.0)
        .map(|g| (g * 1024.0 * 1024.0 * 1024.0) as u64)
        .unwrap_or(DEFAULT_THRESHOLD_BYTES);
    let (owner, prefix) = github::owner_repo(primary_repo)
        .map(|(o, n)| (Some(o), n))
        .unwrap_or((None, "realdata-archive".to_string()));

    let mut catalog = match &catalog_repo {
        Some(repo) => load_catalog(repo).await.unwrap_or_else(|e| {
            eprintln!("realdata.pro: 保管庫の索引を読み込めません(新規として扱います): {e:#}");
            CatalogFile::default()
        }),
        None => CatalogFile::default(),
    };
    if catalog.shards.is_empty() {
        catalog.shards.push(ShardInfo {
            repo: primary_repo.to_string(),
            created_unix: crate::market::now_unix() as i64,
            sealed: false,
        });
    }

    let inner = Inner {
        catalog_repo,
        threshold_bytes,
        new_repo_owner: owner,
        new_repo_prefix: prefix,
        catalog,
        last_size_check: None,
        dirty: false,
    };
    VAULT
        .set(Vault(Mutex::new(inner)))
        .map_err(|_| anyhow::anyhow!("保管庫は既に初期化されています"))?;
    Ok(())
}

pub fn get() -> Option<&'static Vault> {
    VAULT.get()
}

async fn load_catalog(repo: &str) -> Result<CatalogFile> {
    let got = archive::read_many(repo, "HEAD", &[CATALOG_FILE.to_string()]).await?;
    match got.into_iter().next().and_then(|(_, b)| b) {
        Some(bytes) => Ok(serde_json::from_slice(&bytes).unwrap_or_default()),
        None => Ok(CatalogFile::default()),
    }
}

impl Vault {
    /// 書き込み先のシャードを返す(必要なら、まずサイズを確かめて引っ越す)。
    async fn write_repo(&self) -> Result<String> {
        let mut inner = self.0.lock().await;
        self.maybe_rotate(&mut inner).await;
        inner
            .catalog
            .shards
            .iter()
            .rev()
            .find(|s| !s.sealed)
            .or_else(|| inner.catalog.shards.last())
            .map(|s| s.repo.clone())
            .context("保管庫のシャードがありません")
    }

    async fn maybe_rotate(&self, inner: &mut Inner) {
        if inner.catalog_repo.is_none() {
            return; // 索引が無いと、引っ越し先を後から見つけられないので何もしない
        }
        let need_check = match inner.last_size_check {
            Some(t) => t.elapsed() >= SIZE_CHECK_INTERVAL,
            None => true,
        };
        if !need_check {
            return;
        }
        inner.last_size_check = Some(std::time::Instant::now());
        let Some(last) = inner.catalog.shards.last().cloned() else {
            return;
        };
        if last.sealed {
            return;
        }
        match github::repo_size_bytes(&last.repo).await {
            Ok(size) if size >= inner.threshold_bytes => {
                eprintln!(
                    "realdata.pro: 保管庫 {} が上限に近づきました({size} バイト ≥ {} バイト)。新しいシャードへ引っ越します",
                    last.repo, inner.threshold_bytes
                );
                self.rotate(inner).await;
            }
            Ok(_) => {}
            Err(e) => eprintln!(
                "realdata.pro: 保管庫のサイズを確認できません({e:#})。しきい値の判定は次回まで見送ります"
            ),
        }
    }

    async fn rotate(&self, inner: &mut Inner) {
        let n = inner.catalog.shards.len() + 1;
        let name = format!("{}-{n}", inner.new_repo_prefix);
        let new_repo = match &inner.new_repo_owner {
            Some(owner) => match github::create_private_repo(owner, &name).await {
                Ok(repo) => repo,
                Err(e) => {
                    eprintln!(
                        "realdata.pro: 新しいシャードを自動作成できません({e:#})。現在のシャードのまま続けます(手動で対応してください)"
                    );
                    return;
                }
            },
            None => {
                eprintln!("realdata.pro: 新しいシャードの作成先(owner/org)が分かりません。RRD_ARCHIVE_REPO の形式を確認してください");
                return;
            }
        };
        if let Some(last) = inner.catalog.shards.last_mut() {
            last.sealed = true;
        }
        eprintln!("realdata.pro: 新しい保管庫 {new_repo} に切り替えました(以前のシャードは読み込み専用として残ります)");
        inner.catalog.shards.push(ShardInfo {
            repo: new_repo,
            created_unix: crate::market::now_unix() as i64,
            sealed: false,
        });
        inner.dirty = true;
        self.persist(inner).await;
    }

    async fn persist(&self, inner: &mut Inner) {
        let Some(catalog_repo) = inner.catalog_repo.clone() else {
            return;
        };
        if !inner.dirty {
            return;
        }
        let Ok(json) = serde_json::to_vec_pretty(&inner.catalog) else {
            return;
        };
        match archive::push(
            &catalog_repo,
            &[(CATALOG_FILE.to_string(), json)],
            "update catalog",
        )
        .await
        {
            Ok(_) => inner.dirty = false,
            Err(e) => {
                eprintln!("realdata.pro: 保管庫の索引を保存できません(次回また試します): {e:#}")
            }
        }
    }

    /// パスがどのシャードにあるか(索引にある場合のみ)。
    async fn repo_for(&self, path: &str) -> Option<String> {
        self.0.lock().await.catalog.index.get(path).cloned()
    }

    /// パスの場所を記録する(書き込み直後に呼ぶ。失敗しても致命的ではない=次回 [`Store::locate`] が探す)。
    async fn record(&self, path: &str, repo: &str) {
        let mut inner = self.0.lock().await;
        if inner.catalog.index.get(path).map(String::as_str) == Some(repo) {
            return;
        }
        inner
            .catalog
            .index
            .insert(path.to_string(), repo.to_string());
        inner.dirty = true;
        self.persist(&mut inner).await;
    }

    /// パスの索引を消す(削除時)。
    async fn forget(&self, path: &str) {
        let mut inner = self.0.lock().await;
        if inner.catalog.index.remove(path).is_some() {
            inner.dirty = true;
            self.persist(&mut inner).await;
        }
    }

    /// すべてのシャード(新しい順)。
    async fn repos_newest_first(&self) -> Vec<String> {
        let inner = self.0.lock().await;
        inner
            .catalog
            .shards
            .iter()
            .rev()
            .map(|s| s.repo.clone())
            .collect()
    }
}

/// 書き込み先(未初期化なら `fallback` をそのまま返す)。
pub async fn write_repo(fallback: &str) -> String {
    match get() {
        Some(v) => v.write_repo().await.unwrap_or_else(|e| {
            eprintln!("realdata.pro: 保管庫の引っ越し判定に失敗({e:#})。既定の保管庫を使います");
            fallback.to_string()
        }),
        None => fallback.to_string(),
    }
}

/// すべてのシャード(新しい順)。未初期化なら `fallback` の1つだけ。
pub async fn repos_newest_first(fallback: &str) -> Vec<String> {
    match get() {
        Some(v) => v.repos_newest_first().await,
        None => vec![fallback.to_string()],
    }
}

/// 索引にあるパスの場所。未初期化・未記録なら None(呼び出し側が [`repos_newest_first`] で探す)。
pub async fn repo_for(path: &str) -> Option<String> {
    match get() {
        Some(v) => v.repo_for(path).await,
        None => None,
    }
}

/// パスの場所を記録する。
pub async fn record(path: &str, repo: &str) {
    if let Some(v) = get() {
        v.record(path, repo).await;
    }
}

/// パスの索引を消す。
pub async fn forget(path: &str) {
    if let Some(v) = get() {
        v.forget(path).await;
    }
}

/// 索引の場所を優先しつつ、`path` の中身を探して読む(見つからなければ None)。単一シャード運用
/// (索引未設定)なら `fallback` だけを見る。
pub async fn read_first(fallback: &str, path: &str) -> Option<Vec<u8>> {
    let mut candidates: Vec<String> = repo_for(path).await.into_iter().collect();
    for r in repos_newest_first(fallback).await {
        if !candidates.contains(&r) {
            candidates.push(r);
        }
    }
    for repo in candidates {
        if let Ok(got) = archive::read_many(&repo, "HEAD", &[path.to_string()]).await {
            if let Some((_, Some(b))) = got.into_iter().next() {
                return Some(b);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_round_trips_through_json() {
        let c = CatalogFile {
            shards: vec![ShardInfo {
                repo: "https://github.com/aon-co-jp/realdata-archive.git".into(),
                created_unix: 1,
                sealed: false,
            }],
            index: BTreeMap::from([(
                "datasets/a.csv".to_string(),
                "https://github.com/aon-co-jp/realdata-archive.git".to_string(),
            )]),
        };
        let json = serde_json::to_vec(&c).unwrap();
        let back: CatalogFile = serde_json::from_slice(&json).unwrap();
        assert_eq!(back.shards.len(), 1);
        assert_eq!(back.index.len(), 1);
    }
}
