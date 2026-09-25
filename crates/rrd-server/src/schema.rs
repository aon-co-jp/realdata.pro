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
    /// 資金運用・投資の参考情報(公的データ、定期更新)
    pub market: RwLock<crate::market::MarketSnapshot>,
    /// 外貨定期預金の金利(毎朝の自動収集)
    pub deposits: RwLock<crate::deposits::DepositSnapshot>,
    /// 収集中なら true(重複実行を防ぐ)
    pub deposits_running: std::sync::atomic::AtomicBool,
    /// 収集結果などを保存するフォルダ
    pub data_dir: std::path::PathBuf,
    /// aruaru-db による永続化・版管理(RRD_DB_DSN 未設定なら None)
    pub store: Option<crate::store::Store>,
    /// 自動性能検査(CPU / GPU の実測と経路の選択)
    pub tune: crate::tuning::TuneState,
    /// 世界の地名(国・都道府県/州・市区町村/都市)。起動後に裏で読み込む
    pub regions: RwLock<Option<Arc<crate::regions::RegionData>>>,
}

impl AppState {
    pub fn new(
        device: Arc<dyn GpuDevice>,
        llm_base: String,
        data_dir: std::path::PathBuf,
    ) -> AppState {
        let deposits = crate::deposits::load(&data_dir);
        let tune = crate::tuning::TuneState {
            profile: RwLock::new(crate::tuning::load_profile(&data_dir)),
            ..Default::default()
        };
        AppState {
            datasets: RwLock::new(HashMap::new()),
            device,
            http: reqwest::Client::new(),
            llm_base,
            market: RwLock::new(Default::default()),
            deposits: RwLock::new(deposits),
            deposits_running: std::sync::atomic::AtomicBool::new(false),
            data_dir,
            store: None,
            tune,
            regions: RwLock::new(None),
        }
    }
}

/// 外貨定期預金の金利を収集して保存する(実行中なら何もしない)。
pub async fn run_deposit_collection(st: Arc<AppState>) {
    use std::sync::atomic::Ordering;
    if st.deposits_running.swap(true, Ordering::SeqCst) {
        return;
    }
    let snap = crate::deposits::collect(&st.http, &st.llm_base).await;
    if let Err(e) = crate::deposits::save(&st.data_dir, &snap) {
        eprintln!("realdata.pro: 預金金利の保存に失敗: {e:#}");
    }
    eprintln!(
        "realdata.pro: 外貨定期預金の金利を収集しました({}件、エラー{}件)",
        snap.items.len(),
        snap.errors.len()
    );
    if let Ok(mut d) = st.deposits.write() {
        *d = snap;
    }
    st.deposits_running.store(false, Ordering::SeqCst);
}

/// 市場データを取得して反映する。
pub async fn refresh_market(st: &AppState) {
    let (y, m, _, _) = crate::market::jst(crate::market::now_unix());
    let snap = crate::market::snapshot(&st.http, (y, m)).await;
    if let Ok(mut w) = st.market.write() {
        *w = snap;
    }
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
        let st = state(ctx);
        let cpu = st.device.clone();
        with_dataset(ctx, &name, |df| {
            let feats: Vec<&str> = features.iter().map(String::as_str).collect();
            // 処理の大きさ(行列積 k×m · m×k の m×k×k)から、実測に基づいて CPU / GPU を選ぶ
            let k = (feats.len() + 1) as f64;
            let work = df.height() as f64 * k * k;
            let route = crate::tuning::route(st, rrd_tune::Workload::Gemm, work);
            let gpu = match route {
                rrd_tune::Route::Gpu(i) => st.tune.gpus.read().map_err(err_poison)?.get(i).cloned(),
                rrd_tune::Route::Cpu => None,
            };
            let started = std::time::Instant::now();
            let (fit, used_gpu) = match &gpu {
                Some(g) => {
                    match rrd_compute::ols_with(
                        &**g,
                        Some(rrd_tune::SGEMM_SPIRV),
                        df,
                        &target,
                        &feats,
                    ) {
                        Ok(f) => (f, true),
                        // GPU で失敗したら CPU に切り替える(結果は同じ)。理由はログに残す。
                        Err(e) => {
                            eprintln!("realdata.pro: GPU での重回帰に失敗したため CPU で計算します: {e:#}");
                            (
                                rrd_compute::ols(&*cpu, df, &target, &feats).map_err(err)?,
                                false,
                            )
                        }
                    }
                }
                None => (
                    rrd_compute::ols(&*cpu, df, &target, &feats).map_err(err)?,
                    false,
                ),
            };
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            let (id, device_name) = match (&gpu, used_gpu) {
                (Some(g), true) => (route.id(), g.info().name.clone()),
                _ => ("cpu".to_string(), cpu.info().name.clone()),
            };
            st.tune.record(&format!("gemm/{id}"), ms);
            Ok(Regression {
                target: fit.target,
                features: fit.features,
                intercept: fit.intercept,
                coefficients: fit.coefficients,
                r_squared: fit.r_squared,
                n: fit.n,
                device: device_name,
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
        let st = state(ctx);
        let backend = match backend {
            // 自動: 実測で速かった経路を使う。GPU が速い場合も、使えなければ CPU に切り替わる。
            RenderBackend::Auto => match crate::tuning::route(
                st,
                rrd_tune::Workload::Raster,
                f64::from(width) * f64::from(height),
            ) {
                rrd_tune::Route::Cpu => rrd_render::Backend::Cpu,
                rrd_tune::Route::Gpu(_) => rrd_render::Backend::Auto,
            },
            RenderBackend::Gpu => rrd_render::Backend::Gpu,
            RenderBackend::Cpu => rrd_render::Backend::Cpu,
        };
        // GPU 初期化・ラスタライズは重い同期処理なので、非同期実行スレッドを塞がない。
        let r =
            tokio::task::spawn_blocking(move || rrd_render::render(&mesh, width, height, backend))
                .await
                .map_err(|e| Error::new(format!("描画処理が異常終了しました: {e}")))?
                .map_err(Error::new)?;
        st.tune.record(
            &format!(
                "raster/{}",
                if r.backend.starts_with("CPU") {
                    "cpu"
                } else {
                    "gpu0"
                }
            ),
            r.millis,
        );
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

    /// 資金運用・投資の参考情報(政策金利・国債利回り・為替・外貨定期預金金利)。
    async fn market(&self, ctx: &Context<'_>) -> Result<MarketView> {
        let st = state(ctx);
        let market = st.market.read().map_err(err_poison)?.clone();
        let deposits = st.deposits.read().map_err(err_poison)?.clone();
        Ok(MarketView {
            market,
            deposits,
            deposits_collecting: st
                .deposits_running
                .load(std::sync::atomic::Ordering::SeqCst),
        })
    }

    /// 保存と版管理(GitHub の非公開リポジトリ)が使えるか。
    async fn versioning_enabled(&self, ctx: &Context<'_>) -> bool {
        state(ctx).store.is_some()
    }

    /// データセットの版の履歴(新しい順)。
    async fn dataset_versions(
        &self,
        ctx: &Context<'_>,
        name: String,
    ) -> Result<Vec<DatasetVersion>> {
        let store = need_store(ctx)?;
        check_name(&name)?;
        Ok(store
            .versions(Some(&name))
            .await
            .map_err(|e| Error::new(format!("{e:#}")))?
            .into_iter()
            .map(|v| DatasetVersion {
                commit_id: v.commit_id,
                note: v.note,
                created_unix: v.created_unix,
            })
            .collect())
    }

    /// 自動性能検査の結果(構成・実測・決定・割り当て割合・使用実績)。
    async fn compute_profile(&self, ctx: &Context<'_>) -> crate::tuning::ProfileView {
        crate::tuning::view(state(ctx))
    }

    /// 世界リサーチの対象にできる国・地域。
    async fn region_countries(&self, ctx: &Context<'_>) -> Vec<RegionCountryView> {
        let r = state(ctx).regions.read().ok().and_then(|g| g.clone());
        let Some(r) = r else { return vec![] };
        let main: Vec<String> = crate::research::COUNTRIES
            .iter()
            .map(|c| {
                if c.1 == "uk" {
                    "GB".to_string()
                } else {
                    c.1.to_ascii_uppercase()
                }
            })
            .collect();
        let mut v: Vec<RegionCountryView> = r
            .countries
            .iter()
            .map(|c| RegionCountryView {
                code: c.code.clone(),
                name_en: c.name.clone(),
                main: main.contains(&c.code),
            })
            .collect();
        v.sort_by_key(|c| !c.main);
        v
    }

    /// 国を選んだあとの次の階層(parent 空=都道府県・州、指定=その中の市区町村・都市)
    async fn region_children(
        &self,
        ctx: &Context<'_>,
        country: String,
        parent: Option<String>,
    ) -> Result<RegionChildrenView> {
        let r = state(ctx).regions.read().ok().and_then(|g| g.clone());
        let Some(r) = r else {
            return Ok(RegionChildrenView {
                ready: false,
                level: "region".into(),
                label: String::new(),
                items: vec![],
            });
        };
        if r.country(&country).is_none() {
            return Err(Error::new("国コードが正しくありません"));
        }
        let c = match parent.as_deref().filter(|p| !p.is_empty()) {
            Some(p) => r.cities_of(&country, p),
            None => r.top_level(&country),
        };
        Ok(RegionChildrenView {
            ready: true,
            level: c.level,
            label: c.label,
            items: c
                .items
                .into_iter()
                .map(|i| RegionItemView {
                    code: i.code,
                    name: i.name,
                })
                .collect(),
        })
    }

    /// 知りたい情報の一覧
    async fn research_topics(&self) -> Vec<TopicView> {
        crate::places::TOPICS
            .iter()
            .map(|t| TopicView {
                id: t.id.to_string(),
                label: t.label.to_string(),
            })
            .collect()
    }

    async fn research_countries(&self) -> Vec<ResearchCountry> {
        crate::research::COUNTRIES
            .iter()
            .map(|(name, _, _, ja, _)| ResearchCountry {
                name: name.to_string(),
                name_ja: ja.to_string(),
            })
            .collect()
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

fn need_store<'a>(ctx: &Context<'a>) -> Result<&'a crate::store::Store> {
    state(ctx).store.as_ref().ok_or_else(|| {
        Error::new("保存先(GitHub の非公開リポジトリ)が設定されていません。環境変数 RRD_ARCHIVE_REPO を設定してください")
    })
}

fn err_poison<T>(_: T) -> Error {
    Error::new("内部状態へのアクセスに失敗しました")
}

/// データセットを保存する(上限を確認し、同名は上書き)。
fn store_dataset(ctx: &Context<'_>, name: String, df: DataFrame) -> Result<DatasetInfo> {
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
pub struct DatasetVersion {
    pub commit_id: String,
    pub note: String,
    pub created_unix: i64,
}

#[derive(SimpleObject)]
pub struct MarketView {
    pub market: crate::market::MarketSnapshot,
    pub deposits: crate::deposits::DepositSnapshot,
    /// 外貨定期預金の金利を収集中か
    pub deposits_collecting: bool,
}

#[derive(SimpleObject)]
pub struct ResearchCountry {
    /// 英語名(API で指定する値)
    pub name: String,
    pub name_ja: String,
}

#[derive(InputObject)]
pub struct ResearchInput {
    /// 保存するデータセット名
    pub name: String,
    /// 調べたいテーマ(例: 「電気自動車の充電インフラ」)
    #[graphql(default)]
    pub theme: String,
    /// 調べる場所(1〜6)。国全体・都道府県/州・市区町村/都市のどの階層も単独で選べる
    pub places: Vec<PlaceInput>,
    /// 知りたい情報(`researchTopics` の id)
    #[graphql(default)]
    pub topics: Vec<String>,
    /// 現地語に加えて検索に使う言語(最大3つ)
    #[graphql(default)]
    pub search_languages: Vec<String>,
    /// 記事の見出し・要約を先頭のレポート言語へ翻訳して原文と並べる
    #[graphql(default)]
    pub translate_items: bool,
    #[graphql(default = 5)]
    pub per_country: u8,
    #[graphql(default = true)]
    pub include_news: bool,
    #[graphql(default)]
    pub include_github: bool,
    #[graphql(default)]
    pub include_youtube: bool,
    /// レポートの言語(1〜3、先頭の言語で分析し、残りは翻訳)
    pub languages: Vec<String>,
    /// テーマと集めた公開情報を AI へ送ることへの同意
    pub consent: bool,
}

/// 根拠(収集した記事)へのリンク。番号はデータセットの `no` 列と同じ。
#[derive(InputObject)]
pub struct PlaceInput {
    pub country: String,
    pub region: Option<String>,
    pub city: Option<String>,
}

#[derive(SimpleObject)]
pub struct TopicView {
    pub id: String,
    pub label: String,
}

#[derive(SimpleObject)]
pub struct RegionCountryView {
    pub code: String,
    pub name_en: String,
    pub main: bool,
}

#[derive(SimpleObject)]
pub struct RegionItemView {
    pub code: String,
    pub name: String,
}

#[derive(SimpleObject)]
pub struct RegionChildrenView {
    pub ready: bool,
    pub level: String,
    pub label: String,
    pub items: Vec<RegionItemView>,
}

#[derive(SimpleObject, Clone)]
pub struct Evidence {
    pub no: usize,
    pub title: String,
    /// 翻訳した見出し(翻訳版を選んだ場合)
    pub title_tr: Option<String>,
    pub url: Option<String>,
    pub country: String,
}

#[derive(SimpleObject)]
pub struct FindingOut {
    pub title: String,
    pub detail: String,
    pub countries: Vec<String>,
    pub evidence: Vec<Evidence>,
}

#[derive(SimpleObject)]
pub struct ProposalOut {
    pub action: String,
    pub why: String,
    pub priority: String,
    pub evidence: Vec<Evidence>,
}

#[derive(SimpleObject)]
pub struct SentimentOut {
    pub country: String,
    pub score: f64,
    pub reason: String,
}

#[derive(SimpleObject)]
pub struct ResearchReport {
    pub lang: String,
    pub language_name: String,
    pub summary: String,
    pub trends: Vec<FindingOut>,
    pub opportunities: Vec<FindingOut>,
    pub risks: Vec<FindingOut>,
    pub proposals: Vec<ProposalOut>,
    pub sentiment: Vec<SentimentOut>,
    pub provider: Option<String>,
}

#[derive(SimpleObject)]
pub struct QueryUsed {
    pub country: String,
    pub query: String,
}

#[derive(SimpleObject)]
pub struct CountEntry {
    pub label: String,
    pub count: usize,
}

#[derive(SimpleObject)]
pub struct ResearchResult {
    pub dataset: DatasetInfo,
    /// 国ごとに実際に使った検索語
    pub queries: Vec<QueryUsed>,
    /// 集めた記事の一覧(番号は本文中の [n] と同じ)
    pub items: Vec<Evidence>,
    pub reports: Vec<ResearchReport>,
    pub by_country: Vec<CountEntry>,
    pub by_source: Vec<CountEntry>,
    pub warnings: Vec<String>,
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
        store_dataset(ctx, name, df)
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
            dataset: store_dataset(ctx, name, im.df)?,
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
            dataset: store_dataset(ctx, name, im.df)?,
            method: im.method,
        })
    }

    /// データセットを GitHub の非公開リポジトリに保存する。1回の保存が1つの「版」(コミット)になる。
    async fn save_dataset(
        &self,
        ctx: &Context<'_>,
        name: String,
        #[graphql(default)] note: String,
    ) -> Result<DatasetVersion> {
        let store = need_store(ctx)?;
        check_name(&name)?;
        if note.chars().count() > 200 {
            return Err(Error::new("メモは200文字までです"));
        }
        let csv = with_dataset(ctx, &name, |df| Ok(rrd_core::csv::to_csv_string(df)))?;
        let now = crate::market::now_unix() as i64;
        // GitHub の非公開リポジトリへ保存する。1回の保存が1コミット(版)で、メモは履歴に残る
        let commit_id = store
            .save_version(&name, &csv, &note, now)
            .await
            .map_err(|e| Error::new(format!("{e:#}")))?;
        Ok(DatasetVersion {
            commit_id,
            note,
            created_unix: now,
        })
    }

    /// 過去の版のデータセットを into として読み込み直す(コミットを指定して分析を再現)。
    async fn restore_dataset(
        &self,
        ctx: &Context<'_>,
        name: String,
        commit_id: String,
        into: String,
    ) -> Result<DatasetInfo> {
        let store = need_store(ctx)?;
        check_name(&name)?;
        check_name(&into)?;
        let csv = store
            .read_as_of(&name, &commit_id)
            .await
            .map_err(|e| Error::new(format!("{e:#}")))?
            .ok_or_else(|| Error::new("その版にはこのデータセットがありません"))?;
        let df = rrd_core::csv::read_csv_str(&csv).map_err(err)?;
        store_dataset(ctx, into, df)
    }

    /// 性能検査をやり直す(裏で数十秒〜数分かかる。実行中は受け付けない)。
    async fn run_benchmark(&self, ctx: &Context<'_>) -> Result<String> {
        let st = state(ctx).clone();
        if st.tune.running.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok("検査中です。しばらくしてから画面を更新してください".into());
        }
        tokio::spawn(crate::tuning::ensure_profile(st, true));
        Ok("性能検査を開始しました".into())
    }

    /// 性能検査の結果を AI(aruaru-llm)が日本語で解説する。数値は AI へ送るが、機微な情報は含まない。
    async fn explain_profile(&self, ctx: &Context<'_>, consent: bool) -> Result<String> {
        if !consent {
            return Err(Error::new(
                "AI に検査結果(機器の名前と実測値)を送ることへの同意が必要です",
            ));
        }
        let st = state(ctx);
        let p = st
            .tune
            .profile
            .read()
            .map_err(err_poison)?
            .clone()
            .ok_or_else(|| Error::new("性能検査の結果がまだありません"))?;
        let mut prompt = rrd_tune::explain_prompt(&p);
        // aruaru-llm 自身が認識しているアクセラレータ(NPU など)も添える
        if let Ok(resp) = st
            .http
            .get(format!(
                "{}/v1/accelerators",
                st.llm_base.trim_end_matches('/')
            ))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
        {
            if let Ok(j) = resp.json::<serde_json::Value>().await {
                prompt.push_str(&format!(
                    "\naruaru-llm が認識している構成: {}\n",
                    j.get("inventory").map_or(String::new(), |v| v
                        .to_string()
                        .chars()
                        .take(1200)
                        .collect())
                ));
            }
        }
        let (text, _) = crate::explain::complete(&st.http, &st.llm_base, &prompt)
            .await
            .map_err(|e| Error::new(format!("{e:#}")))?;
        Ok(text)
    }

    /// 外貨定期預金の金利をすぐに収集し直す(毎朝の自動収集とは別に)。
    /// 検索と AI の利用回数を守るため、前回の収集から 6 時間以内は受け付けない。
    async fn collect_deposit_rates(&self, ctx: &Context<'_>) -> Result<String> {
        let st = state(ctx).clone();
        let last = st.deposits.read().map_err(err_poison)?.collected_at_unix;
        let now = crate::market::now_unix();
        if st
            .deposits_running
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Ok("収集中です。しばらくしてから画面を更新してください".into());
        }
        if last > 0 && now.saturating_sub(last) < 6 * 3600 {
            return Err(Error::new(
                "前回の収集から6時間以内のため、再収集はできません(毎朝7時に自動で収集します)",
            ));
        }
        tokio::spawn(run_deposit_collection(st));
        Ok("収集を開始しました(数分かかります)".into())
    }

    /// 世界リサーチ: テーマについて各国の情報を集め、AI(aruaru-llm の無料 AI)で分析・提案する。
    /// 集めた記事はデータセット `name` として保存し、ほかの分析にも使える。
    async fn research(&self, ctx: &Context<'_>, input: ResearchInput) -> Result<ResearchResult> {
        if !input.consent {
            return Err(Error::new(
                "テーマと集めた公開情報を AI へ送ることへの同意が必要です",
            ));
        }
        check_name(&input.name)?;
        let st = state(ctx);
        let regions = st
            .regions
            .read()
            .map_err(|_| Error::new("内部エラー"))?
            .clone()
            .ok_or_else(|| {
                Error::new("地名データを準備中です。1分ほど待ってからもう一度お試しください")
            })?;
        let out = crate::research::run(
            &st.http,
            &st.llm_base,
            &regions,
            crate::research::Options {
                theme: input.theme,
                places: input
                    .places
                    .into_iter()
                    .map(|p| crate::places::Place {
                        country: p.country,
                        region: p.region,
                        city: p.city,
                    })
                    .collect(),
                topics: input.topics,
                search_languages: input.search_languages,
                translate_items: input.translate_items,
                per_country: input.per_country,
                include_news: input.include_news,
                include_github: input.include_github,
                include_youtube: input.include_youtube,
                languages: input.languages,
                analyze: true,
                free_only: false,
                use_osm: true,
                pace_ms: 0,
                short_queries: false,
            },
        )
        .await
        .map_err(|e| Error::new(format!("{e:#}")))?;
        let ev = |nums: &[usize]| -> Vec<Evidence> {
            nums.iter()
                .filter_map(|&n| out.items.get(n - 1).map(|it| (n, it)))
                .map(|(n, it)| Evidence {
                    no: n,
                    title: it.title.clone(),
                    title_tr: it.title_tr.clone(),
                    url: Some(it.url.clone()).filter(|u| !u.is_empty()),
                    country: it.country.clone(),
                })
                .collect()
        };
        let findings = |fs: &[crate::research::Finding]| -> Vec<FindingOut> {
            fs.iter()
                .map(|f| FindingOut {
                    title: f.title.clone(),
                    detail: f.detail.clone(),
                    countries: f.countries.clone(),
                    evidence: ev(&f.evidence),
                })
                .collect()
        };
        let reports = out
            .reports
            .iter()
            .map(|r| ResearchReport {
                lang: r.lang.clone(),
                language_name: r.language_name.clone(),
                summary: r.analysis.summary.clone(),
                trends: findings(&r.analysis.trends),
                opportunities: findings(&r.analysis.opportunities),
                risks: findings(&r.analysis.risks),
                proposals: r
                    .analysis
                    .proposals
                    .iter()
                    .map(|p| ProposalOut {
                        action: p.action.clone(),
                        why: p.why.clone(),
                        priority: p.priority.clone(),
                        evidence: ev(&p.evidence),
                    })
                    .collect(),
                sentiment: r
                    .analysis
                    .sentiment
                    .iter()
                    .map(|s| SentimentOut {
                        country: s.country.clone(),
                        score: s.score,
                        reason: s.reason.clone(),
                    })
                    .collect(),
                provider: r.provider.clone(),
            })
            .collect();
        let (by_country, by_source) = crate::research::counts(&out.items);
        let to_entries = |m: std::collections::BTreeMap<String, usize>| {
            let mut v: Vec<CountEntry> = m
                .into_iter()
                .map(|(label, count)| CountEntry { label, count })
                .collect();
            v.sort_by_key(|e| std::cmp::Reverse(e.count));
            v
        };
        Ok(ResearchResult {
            items: out
                .items
                .iter()
                .enumerate()
                .map(|(i, it)| Evidence {
                    no: i + 1,
                    title: it.title.clone(),
                    title_tr: it.title_tr.clone(),
                    url: Some(it.url.clone()).filter(|u| !u.is_empty()),
                    country: it.country.clone(),
                })
                .collect(),
            queries: out
                .queries
                .iter()
                .map(|(c, q)| QueryUsed {
                    country: c.clone(),
                    query: q.clone(),
                })
                .collect(),
            dataset: store_dataset(ctx, input.name, out.df)?,
            reports,
            by_country: to_entries(by_country),
            by_source: to_entries(by_source),
            warnings: out.warnings,
            millis: out.millis,
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
        store_dataset(ctx, into, out)
    }

    /// データセットを削除する。存在していれば true。
    /// 保存済みなら、保存分も削除する(再起動で復活しないように)。過去の版は履歴に残る。
    async fn drop_dataset(&self, ctx: &Context<'_>, name: String) -> Result<bool> {
        let existed = state(ctx)
            .datasets
            .write()
            .map_err(err_poison)?
            .remove(&name)
            .is_some();
        if let Some(store) = &state(ctx).store {
            store
                .delete(&name)
                .await
                .map_err(|e| Error::new(format!("{e:#}")))?;
        }
        Ok(existed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> RrdSchema {
        build_schema(Arc::new(AppState::new(
            rrd_compute::default_device(),
            "http://127.0.0.1:1".into(),
            std::env::temp_dir().join("rrd-schema-test-data"),
        )))
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
