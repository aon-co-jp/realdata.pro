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

pub struct Store {
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
        let pushed = archive::push(
            &self.repo,
            &[(dataset_path(name), csv.as_bytes().to_vec())],
            &message,
        )
        .await?;
        Ok(pushed.commit)
    }

    /// 毎朝の自動収集の結果を保存する。
    pub async fn save_daily(&self, name: &str, csv: &str) -> Result<()> {
        archive::push(
            &self.repo,
            &[(daily_path(name), csv.as_bytes().to_vec())],
            &format!("daily {}", file_safe(name)),
        )
        .await?;
        Ok(())
    }

    /// 保存済みのデータセットを削除する(履歴には残る)。
    pub async fn delete(&self, name: &str) -> Result<()> {
        archive::remove(
            &self.repo,
            &[dataset_path(name)],
            &format!("delete {}", file_safe(name)),
        )
        .await?;
        Ok(())
    }

    /// 起動時に読み込む: データセットの新しいほうから `max_datasets` 件と、毎朝の収集の新しいほうから `max_daily` 件。
    /// (全部を読むと大きくなるので、それ以外は版を指定して `read_as_of` で読む)
    pub async fn load_recent(
        &self,
        max_datasets: usize,
        max_daily: usize,
    ) -> Result<Vec<(String, String)>> {
        let mut paths: Vec<String> = Vec::new();
        let ds = archive::list(&self.repo, "datasets").await?;
        paths.extend(ds.iter().rev().take(max_datasets).cloned());
        let daily = archive::list(&self.repo, "daily").await?;
        // daily/<年>/daily_jp_YYYYMMDD.csv は名前順=日付順
        paths.extend(daily.iter().rev().take(max_daily).cloned());
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for (p, bytes) in archive::read_many(&self.repo, "HEAD", &paths).await? {
            if let Some(b) = bytes {
                if let Ok(text) = String::from_utf8(b) {
                    out.push((stem(&p), text));
                }
            }
        }
        out.sort();
        Ok(out)
    }

    /// 過去の版のデータセット CSV。その版に無ければ None。
    pub async fn read_as_of(&self, name: &str, commit_id: &str) -> Result<Option<String>> {
        if commit_id != "HEAD" && !archive::safe_commit(commit_id) {
            bail!("コミット ID の形式が正しくありません");
        }
        let got = archive::read_many(&self.repo, commit_id, &[dataset_path(name)]).await?;
        Ok(got
            .into_iter()
            .next()
            .and_then(|(_, b)| b)
            .and_then(|b| String::from_utf8(b).ok()))
    }

    /// 版の履歴(新しい順)。
    pub async fn versions(&self, dataset: Option<&str>) -> Result<Vec<Version>> {
        let Some(name) = dataset else {
            bail!("データセット名を指定してください");
        };
        let log = archive::log(&self.repo, &dataset_path(name), 100).await?;
        Ok(log
            .into_iter()
            .map(|(commit_id, at, msg)| Version {
                commit_id,
                dataset: name.to_string(),
                note: msg
                    .split_once("\n\n")
                    .map_or("", |x| x.1)
                    .trim()
                    .to_string(),
                created_unix: at,
            })
            .collect())
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
