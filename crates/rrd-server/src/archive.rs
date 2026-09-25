//! GitHub の非公開リポジトリを「データベース」として読み書きする(VPS には保存しない)。
//!
//! 環境変数 `RRD_ARCHIVE_REPO`(例: `https://github.com/aon-co-jp/realdata-archive.git`)を設定すると、
//! データセット・毎朝の自動収集の結果は、すべてこのリポジトリに保存し、ここから読み込む。
//! 認証は VPS の git の設定(credential store 等)に任せ、トークンはこのプログラムでは扱わない。
//! リポジトリの履歴(git log)が、そのまま「版の履歴」になる。
//!
//! リポジトリは大きくなっていくので、毎回**全部を取得しない**:
//! - 書き込み: 履歴なし(--depth 1)・ファイルの中身なし(--filter=blob:none)・書き込むフォルダだけ
//!   (sparse-checkout)で取得し、ファイルを置いてコミット・push したら、一時フォルダごと消す。
//! - 読み込み: コミットと木だけ(--filter=blob:none、履歴つき)を取得し、必要なファイルの中身だけを
//!   その場で取りに行く(`git show`)。
//!
//! どちらも VPS に残るのは処理中の一時フォルダだけで、終われば消える。

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use tokio::process::Command;

pub fn repo_from_env() -> Option<String> {
    std::env::var("RRD_ARCHIVE_REPO")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 一時フォルダ。捨てるときに中身ごと消す。
struct Tmp(PathBuf);

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tmp_dir() -> PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "rrd-archive-{}-{}-{}",
        std::process::id(),
        crate::market::now_unix(),
        N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ))
}

async fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .await
        .context("git を実行できません")?;
    if !out.status.success() {
        bail!(
            "git {} に失敗: {}",
            args.first().copied().unwrap_or(""),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

async fn git_bytes(dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .await
        .context("git を実行できません")?;
    if !out.status.success() {
        bail!(
            "git {} に失敗: {}",
            args.first().copied().unwrap_or(""),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(out.stdout)
}

/// 保管庫の中の相対パスとして安全か(上位へ出たり、絶対パスにしたりしない)。
pub fn safe_rel(p: &str) -> bool {
    !p.is_empty()
        && !p.starts_with('/')
        && !p.contains('\\')
        && !p.contains(':')
        && p.split('/').all(|c| !c.is_empty() && c != "." && c != "..")
}

/// コミット ID として安全か(16進数のみ)。
pub fn safe_commit(id: &str) -> bool {
    (4..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_hexdigit())
}

async fn clone_to(repo: &str, tmp: &Path, shallow: bool) -> Result<()> {
    let mut args = vec!["clone", "--quiet", "--filter=blob:none", "--no-checkout"];
    if shallow {
        args.extend(["--depth", "1"]);
    }
    let out = Command::new("git")
        .args(&args)
        .arg(repo)
        .arg(tmp)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .await
        .context("git を実行できません")?;
    if !out.status.success() {
        bail!(
            "保管庫を取得できません: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// 保存の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pushed {
    /// 内容が変わったファイル数(0 なら、すでに同じ内容が保存されていた)
    pub changed: usize,
    /// 保存したコミット ID(変化が無ければ、その時点の先頭のコミット ID)
    pub commit: String,
}

async fn prepare(repo: &str, tmp: &Path, top: &[&str]) -> Result<()> {
    clone_to(repo, tmp, true).await?;
    let mut sparse = vec!["sparse-checkout", "set", "--cone"];
    sparse.extend_from_slice(top);
    git(tmp, &sparse).await?;
    git(tmp, &["checkout", "--quiet"]).await?;
    Ok(())
}

async fn commit_and_push(tmp: &Path, message: &str) -> Result<Pushed> {
    git(tmp, &["add", "-A"]).await?;
    let changed = git(tmp, &["status", "--porcelain"])
        .await?
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count();
    if changed == 0 {
        let commit = git(tmp, &["rev-parse", "HEAD"]).await?.trim().to_string();
        return Ok(Pushed { changed: 0, commit });
    }
    git(
        tmp,
        &[
            "-c",
            "user.name=realdata.pro",
            "-c",
            "user.email=noreply@realdata.pro",
            "commit",
            "--quiet",
            "-m",
            message,
        ],
    )
    .await?;
    // 別のプロセスが先に push していたら、ここでは失敗にする(呼び出し側が、取得し直して最初からやり直す)
    git(tmp, &["push", "--quiet", "origin", "HEAD"]).await?;
    let commit = git(tmp, &["rev-parse", "HEAD"]).await?.trim().to_string();
    Ok(Pushed { changed, commit })
}

fn top_dirs(paths: impl Iterator<Item = String>) -> Vec<String> {
    let mut top: Vec<String> = paths
        .filter_map(|p| p.split('/').next().map(String::from))
        .collect();
    top.sort_unstable();
    top.dedup();
    top
}

/// ファイルを保管庫へ保存(追加・上書き)して push する。
pub async fn push(repo: &str, files: &[(String, Vec<u8>)], message: &str) -> Result<Pushed> {
    retry(|| push_once(repo, files, message)).await
}

/// 同時に別のプロセスが push すると拒否されるので、取得し直して最初からやり直す(最大3回)。
async fn retry<F, Fut>(mut f: F) -> Result<Pushed>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Pushed>>,
{
    let mut last = anyhow!("未実行");
    for attempt in 0..3u64 {
        match f().await {
            Ok(p) => return Ok(p),
            Err(e) => {
                last = e;
                tokio::time::sleep(std::time::Duration::from_millis(700 * (attempt + 1))).await;
            }
        }
    }
    Err(last)
}

async fn push_once(repo: &str, files: &[(String, Vec<u8>)], message: &str) -> Result<Pushed> {
    if files.is_empty() {
        bail!("保存するファイルがありません");
    }
    if let Some(bad) = files.iter().find(|(p, _)| !safe_rel(p)) {
        bail!("保管庫へのパスが正しくありません: {}", bad.0);
    }
    let top = top_dirs(files.iter().map(|(p, _)| p.clone()));
    let top: Vec<&str> = top.iter().map(String::as_str).collect();
    let tmp = Tmp(tmp_dir());
    prepare(repo, &tmp.0, &top).await?;
    for (rel, bytes) in files {
        let path = tmp.0.join(rel);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, bytes).with_context(|| format!("{rel} を書けません"))?;
    }
    commit_and_push(&tmp.0, message).await
}

/// 保管庫のファイルを削除して push する(履歴には残る)。
pub async fn remove(repo: &str, paths: &[String], message: &str) -> Result<Pushed> {
    retry(|| remove_once(repo, paths, message)).await
}

async fn remove_once(repo: &str, paths: &[String], message: &str) -> Result<Pushed> {
    if paths.is_empty() {
        bail!("削除するファイルがありません");
    }
    if let Some(bad) = paths.iter().find(|p| !safe_rel(p)) {
        bail!("保管庫へのパスが正しくありません: {bad}");
    }
    let top = top_dirs(paths.iter().cloned());
    let top: Vec<&str> = top.iter().map(String::as_str).collect();
    let tmp = Tmp(tmp_dir());
    prepare(repo, &tmp.0, &top).await?;
    for p in paths {
        git(&tmp.0, &["rm", "-q", "--ignore-unmatch", "--", p]).await?;
    }
    commit_and_push(&tmp.0, message).await
}

/// 履歴つき・中身なしで取得する(読み込み用)。
async fn clone_for_read(repo: &str) -> Result<Tmp> {
    let tmp = Tmp(tmp_dir());
    clone_to(repo, &tmp.0, false).await?;
    Ok(tmp)
}

/// `dir` の下にあるファイルの一覧(相対パス、名前順)。
pub async fn list(repo: &str, dir: &str) -> Result<Vec<String>> {
    if !safe_rel(dir) {
        bail!("保管庫へのパスが正しくありません: {dir}");
    }
    let tmp = clone_for_read(repo).await?;
    let out = git(
        &tmp.0,
        &[
            "ls-tree",
            "-r",
            "--name-only",
            "HEAD",
            "--",
            &format!("{dir}/"),
        ],
    )
    .await
    .unwrap_or_default();
    let mut v: Vec<String> = out.lines().map(String::from).collect();
    v.sort();
    Ok(v)
}

/// 複数のファイルの中身を読む(`rev` は "HEAD" または 16進数のコミット ID)。無いファイルは None。
pub async fn read_many(
    repo: &str,
    rev: &str,
    paths: &[String],
) -> Result<Vec<(String, Option<Vec<u8>>)>> {
    if rev != "HEAD" && !safe_commit(rev) {
        bail!("コミット ID の形式が正しくありません");
    }
    if let Some(bad) = paths.iter().find(|p| !safe_rel(p)) {
        bail!("保管庫へのパスが正しくありません: {bad}");
    }
    let tmp = clone_for_read(repo).await?;
    let mut out = Vec::new();
    for p in paths {
        let bytes = git_bytes(&tmp.0, &["show", &format!("{rev}:{p}")])
            .await
            .ok();
        out.push((p.clone(), bytes));
    }
    Ok(out)
}

/// ファイルを変更したコミットの履歴(新しい順): (コミット ID, UNIX 時刻, メッセージ)。
pub async fn log(repo: &str, path: &str, limit: usize) -> Result<Vec<(String, i64, String)>> {
    if !safe_rel(path) {
        bail!("保管庫へのパスが正しくありません: {path}");
    }
    let tmp = clone_for_read(repo).await?;
    let out = git(
        &tmp.0,
        &[
            "log",
            "-n",
            &limit.clamp(1, 200).to_string(),
            "--format=%H%x1f%at%x1f%B%x1e",
            "--",
            path,
        ],
    )
    .await
    .unwrap_or_default();
    Ok(out
        .split('\u{1e}')
        .filter_map(|rec| {
            let rec = rec.trim_matches(['\n', '\r']);
            let mut it = rec.splitn(3, '\u{1f}');
            let id = it.next()?.trim().to_string();
            let at = it.next()?.trim().parse().ok()?;
            let msg = it.next().unwrap_or("").trim().to_string();
            safe_commit(&id).then_some((id, at, msg))
        })
        .collect())
}

/// 保管庫に接続できるか(読み取りで確かめる)。
pub async fn check(repo: &str) -> Result<()> {
    let out = Command::new("git")
        .args(["ls-remote", "--exit-code", repo, "HEAD"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .await
        .context("git を実行できません")?;
    if !out.status.success() {
        bail!(
            "保管庫に接続できません: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn have_git() -> bool {
        std::process::Command::new("git")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    fn run(dir: &Path, args: &[&str]) {
        let o = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }

    #[test]
    fn rejects_unsafe_paths_and_commits() {
        for bad in [
            "",
            "/etc/passwd",
            "../x",
            "a/../b",
            "a//b",
            "a\\b",
            "c:x",
            "./a",
        ] {
            assert!(!safe_rel(bad), "{bad}");
        }
        assert!(safe_rel("daily/2026/daily_jp_20260901.csv"));
        assert!(safe_commit("a1b2c3d4") && !safe_commit("zz") && !safe_commit("abc; rm -rf /"));
    }

    #[tokio::test]
    async fn git_repo_works_as_a_database() {
        if !have_git() {
            return;
        }
        let base = std::env::temp_dir().join(format!("rrd-archive-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let bare = base.join("origin.git");
        std::fs::create_dir_all(&bare).unwrap();
        run(&bare, &["init", "--quiet", "--bare", "-b", "main"]);
        // 最初のコミットが無いと clone できないので、種になるコミットを置く
        let seed = base.join("seed");
        std::fs::create_dir_all(&seed).unwrap();
        run(&seed, &["init", "--quiet", "-b", "main"]);
        std::fs::write(seed.join("README.md"), "x").unwrap();
        run(&seed, &["add", "-A"]);
        run(
            &seed,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "--quiet",
                "-m",
                "init",
            ],
        );
        run(&seed, &["remote", "add", "origin", bare.to_str().unwrap()]);
        run(&seed, &["push", "--quiet", "origin", "main"]);
        let repo = bare.to_str().unwrap();

        check(repo).await.unwrap();
        let a = push(
            repo,
            &[("datasets/a.csv".into(), b"1,2\n".to_vec())],
            "save a\n\nnote one",
        )
        .await
        .unwrap();
        assert_eq!(a.changed, 1);
        assert!(safe_commit(&a.commit));
        // 同じ内容: 変化なし
        let again = push(
            repo,
            &[("datasets/a.csv".into(), b"1,2\n".to_vec())],
            "again",
        )
        .await
        .unwrap();
        assert_eq!(again.changed, 0);
        assert_eq!(again.commit, a.commit);
        // 上書きすると版が増え、古い版も読める。別フォルダを足しても先のファイルは消えない
        let a2 = push(
            repo,
            &[("datasets/a.csv".into(), b"9,9\n".to_vec())],
            "save a\n\nnote two",
        )
        .await
        .unwrap();
        push(repo, &[("daily/2026/d.csv".into(), b"x".to_vec())], "daily")
            .await
            .unwrap();

        assert_eq!(
            list(repo, "datasets").await.unwrap(),
            vec!["datasets/a.csv".to_string()]
        );
        let now = read_many(
            repo,
            "HEAD",
            &["datasets/a.csv".into(), "datasets/none.csv".into()],
        )
        .await
        .unwrap();
        assert_eq!(now[0].1.as_deref(), Some(&b"9,9\n"[..]));
        assert!(now[1].1.is_none());
        let old = read_many(repo, &a.commit, &["datasets/a.csv".into()])
            .await
            .unwrap();
        assert_eq!(old[0].1.as_deref(), Some(&b"1,2\n"[..]), "古い版を読める");
        let hist = log(repo, "datasets/a.csv", 10).await.unwrap();
        assert_eq!(hist.len(), 2);
        assert_eq!(hist[0].0, a2.commit);
        assert!(hist[0].2.contains("note two") && hist[1].2.contains("note one"));
        // 削除しても履歴には残る
        remove(repo, &["datasets/a.csv".into()], "delete a")
            .await
            .unwrap();
        assert!(list(repo, "datasets").await.unwrap().is_empty());
        let old = read_many(repo, &a.commit, &["datasets/a.csv".into()])
            .await
            .unwrap();
        assert!(old[0].1.is_some());
        // 悪いコミット ID・パスは断る
        assert!(read_many(repo, "abc; rm", &["a".into()]).await.is_err());
        assert!(push(repo, &[("../x".into(), vec![])], "m").await.is_err());
        // 一時フォルダは残らない
        let prefix = format!("rrd-archive-{}-", std::process::id());
        let leftovers = std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
            .count();
        assert_eq!(leftovers, 0);
        let _ = std::fs::remove_dir_all(&base);
    }
}
