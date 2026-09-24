//! GraphQL スキーマ。realdata.pro の操作はすべてこの単一エンドポイントで行う
//! (エコシステム方針: REST エンドポイントを増やさない)。

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_graphql::{
    Context, EmptySubscription, Enum, Error, InputObject, Object, Result, Schema, SimpleObject,
};
use rrd_compute::GpuDevice;
use rrd_core::{Agg, DataFrame, Fill, Value};

pub type RrdSchema = Schema<QueryRoot, MutationRoot, EmptySubscription>;

/// サーバー全体の状態(メモリ上のデータセットと計算デバイス)。
pub struct AppState {
    pub datasets: RwLock<HashMap<String, DataFrame>>,
    pub device: Arc<dyn GpuDevice>,
    /// aruaru-llm への接続(検索の取り込みと、分析結果の説明に使う)。
    pub http: reqwest::Client,
    pub llm_base: String,
}

pub fn build_schema(state: Arc<AppState>) -> RrdSchema {
    Schema::build(QueryRoot, MutationRoot, EmptySubscription)
        .data(state)
        .limit_depth(8)
        .limit_complexity(2000)
        .finish()
}

fn state<'a>(ctx: &Context<'a>) -> &'a Arc<AppState> {
    ctx.data_unchecked::<Arc<AppState>>()
}

fn with_dataset<T>(
    ctx: &Context<'_>,
    name: &str,
    f: impl FnOnce(&DataFrame) -> Result<T>,
) -> Result<T> {
    let map = state(ctx)
        .datasets
        .read()
        .map_err(|_| Error::new("内部状態の読み取りに失敗しました"))?;
    let df = map
        .get(name)
        .ok_or_else(|| Error::new(format!("データセットがありません: {name}")))?;
    f(df)
}

fn err(e: impl std::fmt::Display) -> Error {
    Error::new(e.to_string())
}

/// 表形式の結果(セルは文字列、欠損は null)。
#[derive(SimpleObject)]
pub struct Table {
    pub columns: Vec<String>,
    pub dtypes: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
    pub total_rows: usize,
}

impl Table {
    fn from_df(df: &DataFrame, limit: usize) -> Table {
        let n = df.height().min(limit);
        Table {
            columns: df.columns().iter().map(|c| c.name.clone()).collect(),
            dtypes: df.columns().iter().map(|c| c.dtype().to_string()).collect(),
            rows: (0..n)
                .map(|i| {
                    df.columns()
                        .iter()
                        .map(|c| match c.get(i) {
                            Value::Null => None,
                            v => Some(v.to_string()),
                        })
                        .collect()
                })
                .collect(),
            total_rows: df.height(),
        }
    }
}

#[derive(SimpleObject)]
pub struct ColumnInfo {
    pub name: String,
    pub dtype: String,
    pub nulls: usize,
}

#[derive(SimpleObject)]
pub struct DatasetInfo {
    pub name: String,
    pub rows: usize,
    pub columns: Vec<ColumnInfo>,
}

impl DatasetInfo {
    fn of(name: &str, df: &DataFrame) -> DatasetInfo {
        DatasetInfo {
            name: name.to_string(),
            rows: df.height(),
            columns: df
                .columns()
                .iter()
                .map(|c| ColumnInfo {
                    name: c.name.clone(),
                    dtype: c.dtype().to_string(),
                    nulls: c.null_count(),
                })
                .collect(),
        }
    }
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum AggFn {
    Count,
    Sum,
    Mean,
    Min,
    Max,
    Median,
    Std,
}

impl From<AggFn> for Agg {
    fn from(a: AggFn) -> Agg {
        match a {
            AggFn::Count => Agg::Count,
            AggFn::Sum => Agg::Sum,
            AggFn::Mean => Agg::Mean,
            AggFn::Min => Agg::Min,
            AggFn::Max => Agg::Max,
            AggFn::Median => Agg::Median,
            AggFn::Std => Agg::Std,
        }
    }
}

#[derive(InputObject, Clone)]
pub struct AggInput {
    pub column: String,
    pub func: AggFn,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum FillMethod {
    Mean,
    Median,
    Value,
}

#[derive(InputObject)]
pub struct FillInput {
    pub column: String,
    pub method: FillMethod,
    /// method = VALUE のときの補完値。
    pub value: Option<String>,
}

/// クレンジング・抽出の手順。記載順ではなく、次の固定順で適用する:
/// dedup → fills → dropNulls → filters → select → sort → limit
#[derive(InputObject, Default)]
pub struct TransformInput {
    #[graphql(default)]
    pub dedup: bool,
    #[graphql(default)]
    pub fills: Vec<FillInput>,
    #[graphql(default)]
    pub drop_nulls: bool,
    /// 例: "sales>=100", "city==Tokyo"
    #[graphql(default)]
    pub filters: Vec<String>,
    pub select: Option<Vec<String>>,
    pub sort_by: Option<String>,
    #[graphql(default)]
    pub descending: bool,
    pub limit: Option<usize>,
}

#[derive(SimpleObject)]
pub struct Regression {
    pub target: String,
    pub features: Vec<String>,
    pub intercept: f64,
    pub coefficients: Vec<f64>,
    pub r_squared: f64,
    pub n: usize,
    /// 行列演算を実行した open-cuda デバイス名。
    pub device: String,
}

fn apply_transform(df: &DataFrame, t: &TransformInput) -> rrd_core::Result<DataFrame> {
    let mut df = if t.dedup {
        df.drop_duplicates()
    } else {
        df.clone()
    };
    for f in &t.fills {
        let how = match f.method {
            FillMethod::Mean => Fill::Mean,
            FillMethod::Median => Fill::Median,
            FillMethod::Value => {
                let v = f.value.clone().unwrap_or_default();
                Fill::Value(v.parse::<i64>().map(Value::Int).unwrap_or(Value::Str(v)))
            }
        };
        df = df.fill_null(&f.column, &how)?;
    }
    if t.drop_nulls {
        df = df.drop_nulls();
    }
    for expr in &t.filters {
        df = df.filter_expr(expr)?;
    }
    if let Some(cols) = &t.select {
        df = df.select(&cols.iter().map(String::as_str).collect::<Vec<_>>())?;
    }
    if let Some(col) = &t.sort_by {
        df = df.sort_by(col, !t.descending)?;
    }
    if let Some(n) = t.limit {
        df = df.head(n);
    }
    Ok(df)
}

/// データセット名の制約(表示・将来の aruaru-db テーブル名との対応のため)。
fn check_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if ok {
        Ok(())
    } else {
        Err(Error::new(
            "データセット名は英数字・_・- の1〜64文字にしてください",
        ))
    }
}

/// メモリ保護: 保持できるデータセット数の上限。
const MAX_DATASETS: usize = 64;

pub struct QueryRoot;

#[Object]
impl QueryRoot {
    /// 読み込み済みのデータセット一覧。
    async fn datasets(&self, ctx: &Context<'_>) -> Result<Vec<DatasetInfo>> {
        let map = state(ctx).datasets.read().map_err(err_poison)?;
        let mut v: Vec<DatasetInfo> = map.iter().map(|(n, df)| DatasetInfo::of(n, df)).collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(v)
    }

    /// 先頭の行を表示する。
    async fn preview(
        &self,
        ctx: &Context<'_>,
        name: String,
        #[graphql(default = 20)] limit: usize,
    ) -> Result<Table> {
        with_dataset(ctx, &name, |df| Ok(Table::from_df(df, limit.min(1000))))
    }

    /// 各列の要約統計。
    async fn describe(&self, ctx: &Context<'_>, name: String) -> Result<Table> {
        with_dataset(ctx, &name, |df| {
            let d = df.describe();
            Ok(Table::from_df(&d, d.height()))
        })
    }

    /// キー列でグループ化して集計する。
    async fn group_by(
        &self,
        ctx: &Context<'_>,
        name: String,
        key: String,
        aggs: Vec<AggInput>,
    ) -> Result<Table> {
        with_dataset(ctx, &name, |df| {
            let specs: Vec<(&str, Agg)> = aggs
                .iter()
                .map(|a| (a.column.as_str(), a.func.into()))
                .collect();
            let g = df.group_by(&key, &specs).map_err(err)?;
            Ok(Table::from_df(&g, g.height()))
        })
    }

    /// 2つの数値列のピアソン相関(計算できない場合は null)。
    async fn correlation(
        &self,
        ctx: &Context<'_>,
        name: String,
        a: String,
        b: String,
    ) -> Result<Option<f64>> {
        with_dataset(ctx, &name, |df| df.correlation(&a, &b).map_err(err))
    }

    /// 重回帰(open-cuda の GEMM で正規方程式を組み立てて解く)。
    async fn regression(
        &self,
        ctx: &Context<'_>,
        name: String,
        target: String,
        features: Vec<String>,
    ) -> Result<Regression> {
        let device = state(ctx).device.clone();
        with_dataset(ctx, &name, |df| {
            let feats: Vec<&str> = features.iter().map(String::as_str).collect();
            let fit = rrd_compute::ols(&*device, df, &target, &feats).map_err(err)?;
            Ok(Regression {
                target: fit.target,
                features: fit.features,
                intercept: fit.intercept,
                coefficients: fit.coefficients,
                r_squared: fit.r_squared,
                n: fit.n,
                device: device.info().name.clone(),
            })
        })
    }

    /// 計算に使う open-cuda デバイス名。
    async fn compute_device(&self, ctx: &Context<'_>) -> String {
        state(ctx).device.info().name.clone()
    }

    /// 集計結果のグラフを画像(PNG)で描く。open-directx(GPU)で描き、GPU が無ければ
    /// AVX-512 / AVX2 の CPU ラスタライザで描く。文字(ラベル・凡例)は画像に含めない。
    #[allow(clippy::too_many_arguments)]
    async fn render_chart(
        &self,
        ctx: &Context<'_>,
        name: String,
        key: String,
        agg: AggInput,
        kind: ChartKind,
        #[graphql(default = 640)] width: u32,
        #[graphql(default = 360)] height: u32,
        #[graphql(default_with = "RenderBackend::Auto")] backend: RenderBackend,
    ) -> Result<ChartImage> {
        let values: Vec<f64> = with_dataset(ctx, &name, |df| {
            let g = df
                .group_by(&key, &[(agg.column.as_str(), agg.func.into())])
                .map_err(err)?;
            let col = &g.columns()[1];
            Ok((0..g.height())
                .map(|i| col.get(i).as_f64().unwrap_or(f64::NAN))
                .collect())
        })?;
        if values.len() > 1000 {
            return Err(Error::new("グラフにできる項目は1000件までです"));
        }
        let mesh = match kind {
            ChartKind::Bar => rrd_render::bar_mesh(&values, width, height),
            ChartKind::Pie => rrd_render::pie_mesh(&values, width, height)
                .ok_or_else(|| Error::new("円グラフは、0以上で合計が正の値にだけ使えます"))?,
        };
        let backend = match backend {
            RenderBackend::Auto => rrd_render::Backend::Auto,
            RenderBackend::Gpu => rrd_render::Backend::Gpu,
            RenderBackend::Cpu => rrd_render::Backend::Cpu,
        };
        // GPU 初期化・ラスタライズは重い同期処理なので、非同期実行スレッドを塞がない。
        let r =
            tokio::task::spawn_blocking(move || rrd_render::render(&mesh, width, height, backend))
                .await
                .map_err(|e| Error::new(format!("描画処理が異常終了しました: {e}")))?
                .map_err(Error::new)?;
        use base64::Engine as _;
        Ok(ChartImage {
            png_base64: base64::engine::general_purpose::STANDARD.encode(&r.png),
            width: r.width,
            height: r.height,
            backend: r.backend,
            fallback_reason: r.fallback_reason,
            millis: r.millis,
        })
    }

    /// 説明に使える言語の一覧(世界の主要な約130言語)。
    async fn languages(&self) -> Vec<Language> {
        crate::languages::LANGUAGES
            .iter()
            .map(|(c, ja, native)| Language {
                code: c.to_string(),
                name_ja: ja.to_string(),
                native_name: native.to_string(),
            })
            .collect()
    }
}

fn err_poison<T>(_: T) -> Error {
    Error::new("内部状態へのアクセスに失敗しました")
}

/// データセットを保存する(上限を確認し、同名は上書き)。
fn store(ctx: &Context<'_>, name: String, df: DataFrame) -> Result<DatasetInfo> {
    check_name(&name)?;
    let mut map = state(ctx).datasets.write().map_err(err_poison)?;
    if !map.contains_key(&name) && map.len() >= MAX_DATASETS {
        return Err(Error::new(format!(
            "データセットは最大{MAX_DATASETS}個までです"
        )));
    }
    let info = DatasetInfo::of(&name, &df);
    map.insert(name, df);
    Ok(info)
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum ChartKind {
    Bar,
    Pie,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum RenderBackend {
    /// GPU を試し、使えなければ CPU(AVX-512 / AVX2 / スカラー)。
    Auto,
    Gpu,
    Cpu,
}

#[derive(SimpleObject)]
pub struct ChartImage {
    pub png_base64: String,
    pub width: u32,
    pub height: u32,
    /// 実際に使った描画経路(例: "GPU(open-directx / Vulkan): NVIDIA GeForce GT 730")。
    pub backend: String,
    /// GPU を使えず CPU に切り替えた理由。
    pub fallback_reason: Option<String>,
    pub millis: f64,
}

#[derive(SimpleObject)]
pub struct Language {
    pub code: String,
    pub name_ja: String,
    pub native_name: String,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum SearchSource {
    Google,
    Youtube,
    Github,
}

/// CSV 以外からの取り込み結果。
#[derive(SimpleObject)]
pub struct ImportResult {
    pub dataset: DatasetInfo,
    /// どの方法で表にしたか(例: "HTMLの表(12行)")。
    pub method: String,
}

#[derive(SimpleObject)]
pub struct Explanation {
    pub lang: String,
    pub language_name: String,
    pub text: String,
    pub provider: Option<String>,
}

pub struct MutationRoot;

#[Object]
impl MutationRoot {
    /// CSV テキスト(1行目はヘッダ)を読み込み、名前を付けて保持する。同名は上書き。
    async fn load_csv(&self, ctx: &Context<'_>, name: String, csv: String) -> Result<DatasetInfo> {
        check_name(&name)?;
        let df = rrd_core::csv::read_csv_str(&csv).map_err(err)?;
        store(ctx, name, df)
    }

    /// 検索ワードで Google / YouTube / GitHub を検索し、結果をデータセットにする(aruaru-llm 経由)。
    async fn import_search(
        &self,
        ctx: &Context<'_>,
        name: String,
        source: SearchSource,
        query: String,
        #[graphql(default = 10)] max_results: u8,
    ) -> Result<ImportResult> {
        check_name(&name)?;
        let st = state(ctx);
        let src = match source {
            SearchSource::Google => crate::ingest::SearchSource::Google,
            SearchSource::Youtube => crate::ingest::SearchSource::Youtube,
            SearchSource::Github => crate::ingest::SearchSource::Github,
        };
        let im = crate::ingest::import_search(
            &st.http,
            &st.llm_base,
            src,
            query.trim(),
            max_results.clamp(1, 20),
        )
        .await
        .map_err(|e| Error::new(format!("{e:#}")))?;
        Ok(ImportResult {
            dataset: store(ctx, name, im.df)?,
            method: im.method,
        })
    }

    /// 調査対象の URL を取得して表にする(CSV・JSON・HTML の表、表が無ければリンク一覧)。
    async fn import_url(
        &self,
        ctx: &Context<'_>,
        name: String,
        url: String,
    ) -> Result<ImportResult> {
        check_name(&name)?;
        let im = crate::ingest::import_url(&url)
            .await
            .map_err(|e| Error::new(format!("{e:#}")))?;
        Ok(ImportResult {
            dataset: store(ctx, name, im.df)?,
            method: im.method,
        })
    }

    /// 分析結果を AI(aruaru-llm)が選んだ言語で説明する。
    /// 要約統計と `analysis`(画面の分析結果)を送る。外部 AI へ送られ得るため `consent` が必須。
    async fn explain(
        &self,
        ctx: &Context<'_>,
        name: String,
        languages: Vec<String>,
        consent: bool,
        analysis: Option<String>,
        #[graphql(default = false)] include_sample: bool,
    ) -> Result<Vec<Explanation>> {
        if !consent {
            return Err(Error::new("AI に分析内容を送ることへの同意が必要です"));
        }
        let brief = with_dataset(ctx, &name, |df| {
            crate::explain::build_brief(&name, df, analysis.as_deref(), include_sample)
                .map_err(|e| Error::new(e.to_string()))
        })?;
        let st = state(ctx);
        let out = crate::explain::explain(&st.http, &st.llm_base, &brief, &languages)
            .await
            .map_err(|e| Error::new(format!("{e:#}")))?;
        Ok(out
            .into_iter()
            .map(|x| Explanation {
                lang: x.lang,
                language_name: x.language_name,
                text: x.text,
                provider: x.provider,
            })
            .collect())
    }

    /// クレンジング・抽出を適用した結果を新しいデータセット `into` として保存する。
    async fn transform(
        &self,
        ctx: &Context<'_>,
        source: String,
        into: String,
        steps: TransformInput,
    ) -> Result<DatasetInfo> {
        check_name(&into)?;
        let out = with_dataset(ctx, &source, |df| apply_transform(df, &steps).map_err(err))?;
        store(ctx, into, out)
    }

    /// データセットを削除する。存在していれば true。
    async fn drop_dataset(&self, ctx: &Context<'_>, name: String) -> Result<bool> {
        Ok(state(ctx)
            .datasets
            .write()
            .map_err(err_poison)?
            .remove(&name)
            .is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> RrdSchema {
        build_schema(Arc::new(AppState {
            datasets: RwLock::new(HashMap::new()),
            device: rrd_compute::default_device(),
            http: reqwest::Client::new(),
            llm_base: "http://127.0.0.1:1".into(),
        }))
    }

    async fn run(s: &RrdSchema, q: &str) -> serde_json::Value {
        let r = s.execute(q).await;
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        r.data.into_json().unwrap()
    }

    #[tokio::test]
    async fn load_transform_group_regress() {
        let s = schema();
        run(&s, r#"mutation { loadCsv(name:"d", csv:"k,x,y\na,1,3\na,2,5\nb,3,7\nb,,9\nb,4,9\n") { rows } }"#).await;
        let t = run(&s, r#"mutation { transform(source:"d", into:"c", steps:{dropNulls:true, sortBy:"x", descending:true}) { rows } }"#).await;
        assert_eq!(t["transform"]["rows"], 4);
        let g = run(
            &s,
            r#"{ groupBy(name:"d", key:"k", aggs:[{column:"y", func:SUM}]) { rows } }"#,
        )
        .await;
        assert_eq!(
            g["groupBy"]["rows"],
            serde_json::json!([["a", "8"], ["b", "25"]])
        );
        let r = run(
            &s,
            r#"{ regression(name:"c", target:"y", features:["x"]) { intercept coefficients n } }"#,
        )
        .await;
        assert_eq!(r["regression"]["n"], 4);
        assert!((r["regression"]["coefficients"][0].as_f64().unwrap() - 2.0).abs() < 1e-4);
    }

    #[tokio::test]
    async fn rejects_bad_input() {
        let s = schema();
        assert!(!s
            .execute(r#"mutation { loadCsv(name:"../x", csv:"a\n1\n") { rows } }"#)
            .await
            .errors
            .is_empty());
        assert!(!s
            .execute(r#"{ describe(name:"missing") { rows } }"#)
            .await
            .errors
            .is_empty());
        assert!(!s
            .execute(r#"mutation { loadCsv(name:"x", csv:"a,b\n1\n") { rows } }"#)
            .await
            .errors
            .is_empty());
    }
}
