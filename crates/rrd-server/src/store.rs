//! aruaru-db による永続化と版管理(Git-on-SQL)。
//!
//! aruaru-db は PostgreSQL の通信方式(pgwire)で話せるので、標準の `tokio-postgres` で接続する。
//! - データセットは `rrd_datasets(name, csv)` に CSV テキストとして保存する。
//! - 「版を記録」は `SELECT aruaru_commit(メッセージ)`(その時点の全テーブルのスナップショット)。
//!   記録したコミット ID は `rrd_versions` に(データセット名・メモ・日時と共に)残す。
//! - 過去の版は `SELECT ... AS OF COMMIT '<id>'` で読み出し、いつでも再現できる。
//!
//! 接続先は環境変数 `RRD_DB_DSN`(例: `host=127.0.0.1 port=5433 user=rrd password=… dbname=aruaru`)。
//! 未設定なら、これまでどおりメモリ上のみで動く。パスワードは DSN の中だけに置き、ログには出さない。

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine as _;
use tokio_postgres::{Client, NoTls};

/// 文字列は Base64 にして保存する。aruaru-db の INSERT 解析は値をカンマで単純に区切り、引用符や
/// 括弧の中も区別しないため(2026-09-24 に発見)、CSV のようにカンマ・引用符・改行を含む文字列は
/// そのままでは保存できない。Base64 は英数字と + / = だけなので影響を受けない。
fn enc(s: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(s.as_bytes())
}

fn dec(s: &str) -> Result<String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .map_err(|e| anyhow!("保存された値を復号できません: {e}"))?;
    String::from_utf8(bytes).map_err(|e| anyhow!("保存された値が UTF-8 ではありません: {e}"))
}

pub struct Store {
    client: Client,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub commit_id: String,
    pub dataset: String,
    pub note: String,
    pub created_unix: i64,
}

/// aruaru-db のコミット ID は英数字・`-`・`_` のみ(SQL に埋め込むため必ず検証する)。
pub fn is_safe_commit_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl Store {
    pub async fn connect(dsn: &str) -> Result<Store> {
        let (client, conn) = tokio_postgres::connect(dsn, NoTls)
            .await
            .map_err(|e| anyhow!("aruaru-db に接続できません: {e}"))?;
        tokio::spawn(async move {
            if let Err(e) = conn.await {
                eprintln!("realdata.pro: aruaru-db との接続が切れました: {e}");
            }
        });
        let store = Store { client };
        store.init().await?;
        Ok(store)
    }

    async fn init(&self) -> Result<()> {
        self.client
            .batch_execute(
                "CREATE TABLE IF NOT EXISTS rrd_datasets (name TEXT PRIMARY KEY, csv TEXT NOT NULL, updated_unix TEXT NOT NULL);\
                 CREATE TABLE IF NOT EXISTS rrd_versions (commit_id TEXT NOT NULL, dataset TEXT NOT NULL, note TEXT NOT NULL, created_unix TEXT NOT NULL);",
            )
            .await
            .context("テーブルを準備できません")?;
        Ok(())
    }

    /// データセットを保存する(同名は置き換え)。
    pub async fn save(&self, name: &str, csv: &str, now_unix: i64) -> Result<()> {
        self.client
            .execute("DELETE FROM rrd_datasets WHERE name = $1", &[&name])
            .await
            .context("保存(削除)に失敗")?;
        self.client
            .execute(
                "INSERT INTO rrd_datasets (name, csv, updated_unix) VALUES ($1, $2, $3)",
                &[&name, &enc(csv), &now_unix.to_string()],
            )
            .await
            .context("保存(挿入)に失敗")?;
        Ok(())
    }

    pub async fn delete(&self, name: &str) -> Result<()> {
        self.client
            .execute("DELETE FROM rrd_datasets WHERE name = $1", &[&name])
            .await
            .context("削除に失敗")?;
        Ok(())
    }

    /// 保存済みのデータセットをすべて読む(名前順)。
    pub async fn load_all(&self) -> Result<Vec<(String, String)>> {
        let rows = self
            .client
            .query("SELECT name, csv FROM rrd_datasets", &[])
            .await
            .context("読み込みに失敗")?;
        let mut v: Vec<(String, String)> = rows
            .iter()
            .map(|r| Ok((r.get::<_, String>(0), dec(&r.get::<_, String>(1))?)))
            .collect::<Result<_>>()?;
        v.sort();
        Ok(v)
    }

    /// 現在の全テーブルを「版」として記録し、コミット ID を返す。
    pub async fn commit(&self, message: &str) -> Result<String> {
        let row = self
            .client
            .query_opt("SELECT aruaru_commit($1)", &[&message])
            .await
            .context("版の記録に失敗")?
            .ok_or_else(|| anyhow!("aruaru_commit がコミット ID を返しませんでした"))?;
        let id: String = row
            .try_get(0)
            .map_err(|_| anyhow!("コミット ID を読めません"))?;
        if !is_safe_commit_id(&id) {
            bail!("aruaru-db が想定外の形式のコミット ID を返しました");
        }
        Ok(id)
    }

    pub async fn record_version(
        &self,
        commit_id: &str,
        dataset: &str,
        note: &str,
        now_unix: i64,
    ) -> Result<()> {
        self.client
            .execute(
                "INSERT INTO rrd_versions (commit_id, dataset, note, created_unix) VALUES ($1, $2, $3, $4)",
                &[&commit_id, &dataset, &enc(note), &now_unix.to_string()],
            )
            .await
            .context("版の記録(履歴)に失敗")?;
        Ok(())
    }

    /// 過去の版の一覧(新しい順)。`dataset` が空なら全データセット。
    pub async fn versions(&self, dataset: Option<&str>) -> Result<Vec<Version>> {
        let rows = match dataset {
            Some(d) => self.client.query("SELECT commit_id, dataset, note, created_unix FROM rrd_versions WHERE dataset = $1", &[&d]).await,
            None => self.client.query("SELECT commit_id, dataset, note, created_unix FROM rrd_versions", &[]).await,
        }
        .context("履歴の読み込みに失敗")?;
        let mut v: Vec<Version> = rows
            .iter()
            .map(|r| {
                Ok(Version {
                    commit_id: r.get(0),
                    dataset: r.get(1),
                    note: dec(&r.get::<_, String>(2))?,
                    created_unix: r.get::<_, String>(3).parse().unwrap_or(0),
                })
            })
            .collect::<Result<_>>()?;
        v.sort_by_key(|x| std::cmp::Reverse(x.created_unix));
        Ok(v)
    }

    /// 過去の版(`AS OF COMMIT`)のデータセット CSV。その版に無ければ None。
    pub async fn read_as_of(&self, name: &str, commit_id: &str) -> Result<Option<String>> {
        if !is_safe_commit_id(commit_id) {
            bail!("コミット ID の形式が正しくありません");
        }
        let sql =
            format!("SELECT csv FROM rrd_datasets WHERE name = $1 AS OF COMMIT '{commit_id}'");
        let rows = self
            .client
            .query(&sql, &[&name])
            .await
            .context("過去の版を読めません")?;
        rows.first()
            .map(|r| dec(&r.get::<_, String>(0)))
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_id_validation_blocks_sql() {
        assert!(is_safe_commit_id("a1b2c3-D_4"));
        for bad in [
            "",
            "x'; DROP TABLE rrd_datasets; --",
            "a b",
            "a;b",
            &"x".repeat(200),
        ] {
            assert!(!is_safe_commit_id(bad), "{bad}");
        }
    }

    /// 実際の aruaru-db がある環境でのみ実行(`RRD_TEST_DSN` 未設定なら省略)。
    #[tokio::test]
    async fn roundtrip_with_real_aruaru_db_when_available() {
        let Ok(dsn) = std::env::var("RRD_TEST_DSN") else {
            eprintln!("RRD_TEST_DSN 未設定のため省略");
            return;
        };
        let s = Store::connect(&dsn).await.unwrap();
        let name = format!("t_{}", std::process::id());
        s.save(&name, "a,b\n1,2\n", 1).await.unwrap();
        let c1 = s.commit("test v1").await.unwrap();
        s.record_version(&c1, &name, "v1", 1).await.unwrap();
        s.save(&name, "a,b\n1,2\n3,4\n", 2).await.unwrap();
        let c2 = s.commit("test v2").await.unwrap();
        s.record_version(&c2, &name, "v2", 2).await.unwrap();
        assert_ne!(c1, c2);
        // 現在は2行、v1 の版は1行
        assert_eq!(
            s.load_all()
                .await
                .unwrap()
                .iter()
                .find(|(n, _)| *n == name)
                .unwrap()
                .1,
            "a,b\n1,2\n3,4\n"
        );
        assert_eq!(
            s.read_as_of(&name, &c1).await.unwrap().as_deref(),
            Some("a,b\n1,2\n")
        );
        assert_eq!(
            s.read_as_of(&name, &c2).await.unwrap().as_deref(),
            Some("a,b\n1,2\n3,4\n")
        );
        let vs = s.versions(Some(&name)).await.unwrap();
        assert_eq!(
            vs.iter().map(|v| v.note.as_str()).collect::<Vec<_>>(),
            ["v2", "v1"]
        );
        s.delete(&name).await.unwrap();
        assert!(s.load_all().await.unwrap().iter().all(|(n, _)| *n != name));
        assert!(s.read_as_of(&name, "bad'id").await.is_err());
    }
}
