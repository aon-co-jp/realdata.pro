//! 複数の GitHub 非公開リポジトリ(シャード)を跨いで「1つの保管庫」に見せる。
//!
//! GitHub は1リポジトリあたり10GBを推奨上限としている(超えると警告・将来の制限の対象になりうる)。
//! `RRD_ARCHIVE_REPO` の1リポジトリだけに保存し続けるとこの上限に近づいていくため、現在書き込み中の
//! シャードのサイズを定期的に確かめ(GitHub API)、しきい値(既定1GB、2026-10-03に8GBから変更)を超えたら、あらかじめ用意して
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
/// このバイト数を超えたら次のシャードへ引っ越す。
/// 2026-10-03変更(ユーザー指示「推薦の1GBに下げて」): 8GB → 1GB。GitHub公式の推奨は
/// 「1リポジトリ1GB未満」、ソフト上限は5GB(旧既定の8GBは上限を超えていた)。
const DEFAULT_THRESHOLD_BYTES: u64 = 1024 * 1024 * 1024;
/// しきい値に達するまでの見込みがこの日数以内になったら、次のシャードを先行作成する
/// (切り替え自体はしきい値に達してから。ユーザー指示「溢れる前に予測して…あらかじめ作っておいて」)。
const PRECREATE_WITHIN_DAYS: f64 = 14.0;

/// 増加ペース(作成からの平均、バイト/秒)から、しきい値に達するまでの日数を見積もる。
/// 作成直後や増えていない(ペースが0以下)場合は見積もれないので`None`。
/// 既にしきい値以上なら`Some(0.0)`。
fn predict_days_to_threshold(size: u64, age_secs: u64, threshold: u64) -> Option<f64> {
    if size >= threshold {
        return Some(0.0);
    }
    if age_secs == 0 || size == 0 {
        return None;
    }
    let rate = size as f64 / age_secs as f64; // バイト/秒
    Some((threshold - size) as f64 / rate / 86_400.0)
}
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
    /// 先行作成を試みた次のシャード名(同じ名前で何度も作成を試みないため。再起動すると忘れるが、
    /// 作成は同名があればそのまま使う(冪等)ので問題ない)
    standby_attempted: Option<String>,
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
        standby_attempted: None,
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
            Ok(size) => {
                // 増加ペースから見込みを立て、しきい値に近づいていたら次のシャードを先に作っておく。
                let age = (crate::market::now_unix() as i64 - last.created_unix).max(0) as u64;
                let predicted = predict_days_to_threshold(size, age, inner.threshold_bytes);
                if let Some(days) = predicted {
                    if days <= PRECREATE_WITHIN_DAYS {
                        let n = inner.catalog.shards.len() + 1;
                        let name = format!("{}-{n}", inner.new_repo_prefix);
                        if inner.standby_attempted.as_deref() != Some(name.as_str()) {
                            inner.standby_attempted = Some(name.clone());
                            if let Some(owner) = inner.new_repo_owner.clone() {
                                eprintln!(
                                    "realdata.pro: 保管庫 {} はあと約{days:.1}日でしきい値に達する見込みです。次のシャード {name} を先行作成します",
                                    last.repo
                                );
                                if let Err(e) = github::create_private_repo(&owner, &name).await {
                                    eprintln!("realdata.pro: 次のシャードの先行作成に失敗しました({e:#})。しきい値到達時に再試行します");
                                }
                            }
                        }
                    }
                }
            }
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
    fn default_threshold_is_the_recommended_one_gib() {
        assert_eq!(DEFAULT_THRESHOLD_BYTES, 1024 * 1024 * 1024);
    }

    #[test]
    fn predict_days_to_threshold_uses_average_growth_since_creation() {
        const GIB: u64 = 1024 * 1024 * 1024;
        const DAY: u64 = 86_400;
        // 10日で0.5GiB増えた → 毎日0.05GiB → 残り0.5GiBまで10日。
        let days = predict_days_to_threshold(GIB / 2, 10 * DAY, GIB).unwrap();
        assert!((days - 10.0).abs() < 0.01, "got {days}");
        // しきい値以上なら0日。
        assert_eq!(predict_days_to_threshold(GIB, DAY, GIB), Some(0.0));
        assert_eq!(predict_days_to_threshold(2 * GIB, DAY, GIB), Some(0.0));
        // 作成直後(年齢0)や、まだ何も入っていない(サイズ0)場合は見積もれない。
        assert_eq!(predict_days_to_threshold(1000, 0, GIB), None);
        assert_eq!(predict_days_to_threshold(0, 10 * DAY, GIB), None);
    }

    #[test]
    fn precreate_window_triggers_only_when_close() {
        const GIB: u64 = 1024 * 1024 * 1024;
        const DAY: u64 = 86_400;
        // 現状(482KBが作成から約8日): 到達まで非常に長く、先行作成しない。
        let far = predict_days_to_threshold(482 * 1024, 8 * DAY, GIB).unwrap();
        assert!(far > PRECREATE_WITHIN_DAYS, "got {far}");
        // 30日で0.9GiB → 残り0.1GiBまで約3.3日、先行作成する。
        let near = predict_days_to_threshold(GIB * 9 / 10, 30 * DAY, GIB).unwrap();
        assert!(near <= PRECREATE_WITHIN_DAYS, "got {near}");
    }

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
