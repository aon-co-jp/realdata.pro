//! データセットの保存と版管理。保存先は GitHub の非公開リポジトリ(VPS の DB・ディスクには保存しない)。
//!
//! 環境変数 `RRD_ARCHIVE_REPO`(例: `https://github.com/aon-co-jp/realdata-archive.git`)で指定する。
//! - データセットは `datasets/<名前>.csv`、毎朝の自動収集は `daily/<年>/<名前>.csv` に保存する。
//! - 「保存」は 1 回が 1 コミット。コミットのメッセージが「名前 + メモ」で、`git log` がそのまま版の履歴になる。
//! - 過去の版は、コミット ID を指定して読み出せる(`git show <commit>:<path>`)。
//!
//! 読み書きの詳しい仕組み(VPS に何も残さない取得方法)は [`crate::archive`] を参照。

use anyhow::{bail, Result};

use crate::archive;
use crate::vault;

pub struct Store {
    /// 最初のシャード(保管庫の索引 `vault` が未初期化のときの既定の読み書き先として使う)。
    repo: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub commit_id: String,
    pub dataset: String,
    pub note: String,
    pub created_unix: i64,
}

/// ファイル名に使える形にする(日本語などの文字はそのまま、記号は `_` に)
pub fn file_safe(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '_' | '-' | '.') {
                c
            } else {
                '_'
            }
        })
        .take(100)
        .collect();
    if s.is_empty() || s.starts_with('.') {
        format!("_{s}")
    } else {
        s
    }
}

fn dataset_path(name: &str) -> String {
    format!("datasets/{}.csv", file_safe(name))
}

/// 「daily_jp_YYYYMMDD」の年で `daily/<年>/` に分ける(それ以外の名前は `daily/other/`)。
fn daily_path(name: &str) -> String {
    let year = name
        .strip_prefix("daily_jp_")
        .filter(|s| s.len() == 8 && s.bytes().all(|b| b.is_ascii_digit()))
        .map_or("other", |s| &s[..4]);
    format!("daily/{year}/{}.csv", file_safe(name))
}

fn stem(path: &str) -> String {
    path.rsplit('/')
        .next()
        .unwrap_or(path)
        .trim_end_matches(".csv")
        .to_string()
}

impl Store {
    pub async fn connect(repo: &str) -> Result<Store> {
        archive::check(repo).await?;
        Ok(Store {
            repo: repo.to_string(),
        })
    }

    /// データセットを保存し、その版(コミット ID)を返す。`note` は版の履歴に残るメモ。
    ///
    /// 書き込み先のシャードは [`vault`] が決める(容量が上限に近づいていれば、新しいシャードへ自動で
    /// 引っ越す)。保管庫の索引(`RRD_CATALOG_REPO`)が無い環境では、これまでどおり単一シャードに書く。
    pub async fn save_version(
        &self,
        name: &str,
        csv: &str,
        note: &str,
        _now_unix: i64,
    ) -> Result<String> {
        let note: String = note
            .chars()
            .filter(|c| *c != '\u{1e}' && *c != '\u{1f}')
            .collect();
        let message = format!("save {}\n\n{}", file_safe(name), note.trim());
        let path = dataset_path(name);
        let repo = vault::write_repo(&self.repo).await;
        let pushed =
            archive::push(&repo, &[(path.clone(), csv.as_bytes().to_vec())], &message).await?;
        vault::record(&path, &repo).await;
        Ok(pushed.commit)
    }

    /// 毎朝の自動収集の結果を保存する。
    pub async fn save_daily(&self, name: &str, csv: &str) -> Result<()> {
        let path = daily_path(name);
        let repo = vault::write_repo(&self.repo).await;
        archive::push(
            &repo,
            &[(path.clone(), csv.as_bytes().to_vec())],
            &format!("daily {}", file_safe(name)),
        )
        .await?;
        vault::record(&path, &repo).await;
        Ok(())
    }

    /// 保存済みのデータセットを削除する(履歴には残る)。
    pub async fn delete(&self, name: &str) -> Result<()> {
        let path = dataset_path(name);
        let repo = self.locate(&path).await?;
        archive::remove(
            &repo,
            std::slice::from_ref(&path),
            &format!("delete {}", file_safe(name)),
        )
        .await?;
        vault::forget(&path).await;
        Ok(())
    }

    /// パスがあるシャードを探す(索引にあればそこ、無ければ新しい順にシャードを探す)。
    async fn locate(&self, path: &str) -> Result<String> {
        if let Some(r) = vault::repo_for(path).await {
            return Ok(r);
        }
        for repo in vault::repos_newest_first(&self.repo).await {
            if let Ok(got) = archive::read_many(&repo, "HEAD", &[path.to_string()]).await {
                if got.into_iter().next().is_some_and(|(_, b)| b.is_some()) {
                    return Ok(repo);
                }
            }
        }
        bail!("{path} が保管庫に見つかりません")
    }

    /// 起動時に読み込む: データセットの新しいほうから `max_datasets` 件と、毎朝の収集の新しいほうから `max_daily` 件。
    /// (全部を読むと大きくなるので、それ以外は版を指定して `read_as_of` で読む)
    ///
    /// 複数シャードにまたがる場合は、シャードごとにファイル一覧を取り直して束ね、パスごとにシャードを
    /// 決めてからシャード単位でまとめて読む(シャード数だけの通信で済む)。
    pub async fn load_recent(
        &self,
        max_datasets: usize,
        max_daily: usize,
    ) -> Result<Vec<(String, String)>> {
        let repos = vault::repos_newest_first(&self.repo).await;
        let mut ds_set: std::collections::BTreeSet<String> = Default::default();
        let mut daily_set: std::collections::BTreeSet<String> = Default::default();
        for repo in &repos {
            if let Ok(v) = archive::list(repo, "datasets").await {
                ds_set.extend(v);
            }
            if let Ok(v) = archive::list(repo, "daily").await {
                daily_set.extend(v);
            }
        }
        let mut ds: Vec<String> = ds_set.into_iter().collect();
        let mut daily: Vec<String> = daily_set.into_iter().collect();
        ds.sort();
        daily.sort(); // daily/<年>/daily_jp_YYYYMMDD.csv は名前順=日付順
        let mut wanted: Vec<String> = ds.iter().rev().take(max_datasets).cloned().collect();
        wanted.extend(daily.iter().rev().take(max_daily).cloned());
        if wanted.is_empty() {
            return Ok(Vec::new());
        }

        let mut by_repo: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        let mut unresolved: Vec<String> = Vec::new();
        for p in &wanted {
            match vault::repo_for(p).await {
                Some(r) => by_repo.entry(r).or_default().push(p.clone()),
                None => unresolved.push(p.clone()),
            }
        }
        // 索引に無いもの(単一シャード運用時や、索引が付く前のデータ)は、シャードを順に探す
        for p in unresolved {
            if let Ok(r) = self.locate(&p).await {
                by_repo.entry(r).or_default().push(p);
            }
        }

        let mut out = Vec::new();
        for (repo, paths) in by_repo {
            if let Ok(got) = archive::read_many(&repo, "HEAD", &paths).await {
                for (p, bytes) in got {
                    if let Some(b) = bytes {
                        if let Ok(text) = String::from_utf8(b) {
                            out.push((stem(&p), text));
                        }
                    }
                }
            }
        }
        out.sort();
        Ok(out)
    }

    /// 過去の版のデータセット CSV。その版に無ければ None。
    ///
    /// 索引にある(現在の)シャードをまず試し、見つからなければ他のシャードも探す
    /// (引っ越し前に書かれた版は、引っ越し後も元のシャードに残っているため)。
    pub async fn read_as_of(&self, name: &str, commit_id: &str) -> Result<Option<String>> {
        if commit_id != "HEAD" && !archive::safe_commit(commit_id) {
            bail!("コミット ID の形式が正しくありません");
        }
        let path = dataset_path(name);
        let mut candidates: Vec<String> = vault::repo_for(&path).await.into_iter().collect();
        for r in vault::repos_newest_first(&self.repo).await {
            if !candidates.contains(&r) {
                candidates.push(r);
            }
        }
        for repo in candidates {
            if let Ok(got) = archive::read_many(&repo, commit_id, std::slice::from_ref(&path)).await
            {
                if let Some((_, Some(b))) = got.into_iter().next() {
                    return Ok(String::from_utf8(b).ok());
                }
            }
        }
        Ok(None)
    }

    /// 版の履歴(新しい順)。同じ名前のデータセットが引っ越し前後で別々のシャードに書かれていることが
    /// あるため、すべてのシャードから履歴を集めて日時順に束ね直す。
    pub async fn versions(&self, dataset: Option<&str>) -> Result<Vec<Version>> {
        let Some(name) = dataset else {
            bail!("データセット名を指定してください");
        };
        let path = dataset_path(name);
        let mut out = Vec::new();
        for repo in vault::repos_newest_first(&self.repo).await {
            if let Ok(log) = archive::log(&repo, &path, 100).await {
                out.extend(log.into_iter().map(|(commit_id, at, msg)| {
                    Version {
                        commit_id,
                        dataset: name.to_string(),
                        note: msg
                            .split_once("\n\n")
                            .map_or("", |x| x.1)
                            .trim()
                            .to_string(),
                        created_unix: at,
                    }
                }));
            }
        }
        out.sort_by_key(|v| std::cmp::Reverse(v.created_unix));
        out.truncate(100);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_names() {
        assert_eq!(
            dataset_path("research_20260925_山梨県"),
            "datasets/research_20260925_山梨県.csv"
        );
        assert_eq!(dataset_path("../a b"), "datasets/_.._a_b.csv");
        assert_eq!(
            daily_path("daily_jp_20260925"),
            "daily/2026/daily_jp_20260925.csv"
        );
        assert_eq!(daily_path("weird"), "daily/other/weird.csv");
        assert_eq!(
            stem("daily/2026/daily_jp_20260925.csv"),
            "daily_jp_20260925"
        );
        assert!(
            archive::safe_rel(&dataset_path("a/b")),
            "スラッシュは名前の一部にならない"
        );
    }
}
