//! DataFrame: 同じ長さの列の集合。読込後のクレンジング・探索・集計を行う。

use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;

use crate::column::{Column, ColumnData, Value};
use crate::error::{Error, Result};
use crate::stats;

/// 比較演算子(filter用)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CmpOp {
    fn test(self, ord: Ordering) -> bool {
        match self {
            CmpOp::Eq => ord == Ordering::Equal,
            CmpOp::Ne => ord != Ordering::Equal,
            CmpOp::Lt => ord == Ordering::Less,
            CmpOp::Le => ord != Ordering::Greater,
            CmpOp::Gt => ord == Ordering::Greater,
            CmpOp::Ge => ord != Ordering::Less,
        }
    }
}

/// 集計関数(group_by用)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agg {
    Count,
    Sum,
    Mean,
    Min,
    Max,
    Median,
    Std,
}

impl Agg {
    pub fn parse(s: &str) -> Result<Agg> {
        Ok(match s.to_ascii_lowercase().as_str() {
            "count" => Agg::Count,
            "sum" => Agg::Sum,
            "mean" | "avg" => Agg::Mean,
            "min" => Agg::Min,
            "max" => Agg::Max,
            "median" => Agg::Median,
            "std" => Agg::Std,
            other => return Err(Error::Invalid(format!("未知の集計関数: {other}"))),
        })
    }

    fn name(self) -> &'static str {
        match self {
            Agg::Count => "count",
            Agg::Sum => "sum",
            Agg::Mean => "mean",
            Agg::Min => "min",
            Agg::Max => "max",
            Agg::Median => "median",
            Agg::Std => "std",
        }
    }

    fn apply(self, col: &Column, rows: &[usize]) -> Option<f64> {
        if self == Agg::Count {
            return Some(rows.iter().filter(|&&i| !col.is_null(i)).count() as f64);
        }
        let v = col.take(rows).numeric_values();
        match self {
            Agg::Count => unreachable!(),
            Agg::Sum => Some(stats::sum(&v)),
            Agg::Mean => stats::mean(&v),
            Agg::Min => stats::min(&v),
            Agg::Max => stats::max(&v),
            Agg::Median => stats::median(&v),
            Agg::Std => stats::std_dev(&v),
        }
    }
}

/// 欠損値の補完方法。
#[derive(Debug, Clone, PartialEq)]
pub enum Fill {
    Mean,
    Median,
    Value(Value),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DataFrame {
    columns: Vec<Column>,
}

impl DataFrame {
    /// 列の長さと名前の重複を検証して作る。
    pub fn new(columns: Vec<Column>) -> Result<Self> {
        if let Some(first) = columns.first() {
            if let Some(bad) = columns.iter().find(|c| c.len() != first.len()) {
                return Err(Error::Invalid(format!(
                    "列{}の長さが他の列と異なります",
                    bad.name
                )));
            }
        }
        for (i, c) in columns.iter().enumerate() {
            if columns[..i].iter().any(|d| d.name == c.name) {
                return Err(Error::Invalid(format!("列名が重複しています: {}", c.name)));
            }
        }
        Ok(Self { columns })
    }

    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    pub fn height(&self) -> usize {
        self.columns.first().map_or(0, Column::len)
    }

    pub fn width(&self) -> usize {
        self.columns.len()
    }

    pub fn column(&self, name: &str) -> Result<&Column> {
        self.columns
            .iter()
            .find(|c| c.name == name)
            .ok_or_else(|| Error::ColumnNotFound(name.to_string()))
    }

    fn take_rows(&self, idx: &[usize]) -> DataFrame {
        DataFrame {
            columns: self.columns.iter().map(|c| c.take(idx)).collect(),
        }
    }

    /// 先頭n行。
    pub fn head(&self, n: usize) -> DataFrame {
        let idx: Vec<usize> = (0..self.height().min(n)).collect();
        self.take_rows(&idx)
    }

    /// 指定列のみを残す。
    pub fn select(&self, names: &[&str]) -> Result<DataFrame> {
        let cols = names
            .iter()
            .map(|n| self.column(n).cloned())
            .collect::<Result<Vec<_>>>()?;
        DataFrame::new(cols)
    }

    /// `列 演算子 値` を満たす行だけを残す(欠損値の行は常に除外)。
    pub fn filter(&self, name: &str, op: CmpOp, rhs: &Value) -> Result<DataFrame> {
        let col = self.column(name)?;
        let rhs_num = match rhs {
            Value::Str(s) if col.is_numeric() => {
                s.parse::<f64>().map(Value::Float).map_err(|_| {
                    Error::Invalid(format!("数値列{name}と文字列'{s}'は比較できません"))
                })?
            }
            v => v.clone(),
        };
        let idx: Vec<usize> = (0..self.height())
            .filter(|&i| {
                let v = col.get(i);
                v != Value::Null && op.test(v.total_cmp(&rhs_num))
            })
            .collect();
        Ok(self.take_rows(&idx))
    }

    /// 文字列式 `col>=10` / `city==Tokyo` 等でフィルタする。
    pub fn filter_expr(&self, expr: &str) -> Result<DataFrame> {
        const OPS: [(&str, CmpOp); 6] = [
            ("==", CmpOp::Eq),
            ("!=", CmpOp::Ne),
            ("<=", CmpOp::Le),
            (">=", CmpOp::Ge),
            ("<", CmpOp::Lt),
            (">", CmpOp::Gt),
        ];
        for (tok, op) in OPS {
            if let Some((l, r)) = expr.split_once(tok) {
                return self.filter(l.trim(), op, &Value::Str(r.trim().to_string()));
            }
        }
        Err(Error::Parse(format!("フィルタ式を解釈できません: {expr}")))
    }

    /// 列で並べ替える(安定ソート、欠損は昇順で先頭)。
    pub fn sort_by(&self, name: &str, ascending: bool) -> Result<DataFrame> {
        let col = self.column(name)?;
        let mut idx: Vec<usize> = (0..self.height()).collect();
        idx.sort_by(|&a, &b| {
            let o = col.get(a).total_cmp(&col.get(b));
            if ascending {
                o
            } else {
                o.reverse()
            }
        });
        Ok(self.take_rows(&idx))
    }

    /// いずれかの列に欠損を含む行を除く。
    pub fn drop_nulls(&self) -> DataFrame {
        let idx: Vec<usize> = (0..self.height())
            .filter(|&i| self.columns.iter().all(|c| !c.is_null(i)))
            .collect();
        self.take_rows(&idx)
    }

    /// 完全に同一の行を除く(最初の出現を残す)。
    pub fn drop_duplicates(&self) -> DataFrame {
        let mut seen = std::collections::HashSet::new();
        let idx: Vec<usize> = (0..self.height())
            .filter(|&i| {
                let key: Vec<String> = self
                    .columns
                    .iter()
                    .map(|c| format!("{:?}", c.get(i)))
                    .collect();
                seen.insert(key)
            })
            .collect();
        self.take_rows(&idx)
    }

    /// 指定列の欠損を補完する。数値列を平均/中央値で埋めると float 列になる。
    pub fn fill_null(&self, name: &str, how: &Fill) -> Result<DataFrame> {
        let col = self.column(name)?;
        let filled = match how {
            Fill::Mean | Fill::Median => {
                if !col.is_numeric() {
                    return Err(Error::Invalid(format!("{name}は数値列ではありません")));
                }
                let v = col.numeric_values();
                let fillv = if *how == Fill::Mean {
                    stats::mean(&v)
                } else {
                    stats::median(&v)
                };
                let fillv =
                    fillv.ok_or_else(|| Error::Invalid(format!("{name}に値がありません")))?;
                ColumnData::Float(
                    (0..col.len())
                        .map(|i| Some(col.get(i).as_f64().unwrap_or(fillv)))
                        .collect(),
                )
            }
            Fill::Value(v) => match (&col.data, v) {
                (ColumnData::Str(d), v) => ColumnData::Str(
                    d.iter()
                        .map(|x| Some(x.clone().unwrap_or_else(|| v.to_string())))
                        .collect(),
                ),
                (ColumnData::Int(d), Value::Int(f)) => {
                    ColumnData::Int(d.iter().map(|x| Some(x.unwrap_or(*f))).collect())
                }
                (ColumnData::Bool(d), Value::Bool(f)) => {
                    ColumnData::Bool(d.iter().map(|x| Some(x.unwrap_or(*f))).collect())
                }
                (_, v) if col.is_numeric() => {
                    let f = v
                        .as_f64()
                        .or_else(|| v.to_string().parse().ok())
                        .ok_or_else(|| Error::Invalid(format!("{name}を'{v}'で補完できません")))?;
                    ColumnData::Float(
                        (0..col.len())
                            .map(|i| Some(col.get(i).as_f64().unwrap_or(f)))
                            .collect(),
                    )
                }
                _ => return Err(Error::Invalid(format!("{name}を'{v}'で補完できません"))),
            },
        };
        let columns = self
            .columns
            .iter()
            .map(|c| {
                if c.name == name {
                    Column::new(name, filled.clone())
                } else {
                    c.clone()
                }
            })
            .collect();
        DataFrame::new(columns)
    }

    /// 各列の要約統計(探索的データ分析の入口)。
    pub fn describe(&self) -> DataFrame {
        let mut name = Vec::new();
        let mut dtype = Vec::new();
        let mut count = Vec::new();
        let mut nulls = Vec::new();
        let mut cols: [Vec<Option<f64>>; 7] = Default::default();
        for c in &self.columns {
            name.push(Some(c.name.clone()));
            dtype.push(Some(c.dtype().to_string()));
            count.push(Some((c.len() - c.null_count()) as i64));
            nulls.push(Some(c.null_count() as i64));
            let v = if c.is_numeric() {
                c.numeric_values()
            } else {
                Vec::new()
            };
            let vals = [
                stats::mean(&v),
                stats::std_dev(&v),
                stats::min(&v),
                stats::quantile(&v, 0.25),
                stats::median(&v),
                stats::quantile(&v, 0.75),
                stats::max(&v),
            ];
            for (dst, x) in cols.iter_mut().zip(vals) {
                dst.push(x);
            }
        }
        let mut out = vec![
            Column::new("column", ColumnData::Str(name)),
            Column::new("dtype", ColumnData::Str(dtype)),
            Column::new("count", ColumnData::Int(count)),
            Column::new("nulls", ColumnData::Int(nulls)),
        ];
        for (label, data) in ["mean", "std", "min", "25%", "50%", "75%", "max"]
            .into_iter()
            .zip(cols)
        {
            out.push(Column::new(label, ColumnData::Float(data)));
        }
        DataFrame { columns: out }
    }

    /// キー列でグループ化し、(列名, 集計関数) の組で集計する。キーは出現順。
    pub fn group_by(&self, key: &str, aggs: &[(&str, Agg)]) -> Result<DataFrame> {
        let key_col = self.column(key)?;
        let mut order: Vec<Value> = Vec::new();
        let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
        for i in 0..self.height() {
            let k = key_col.get(i);
            let entry = groups.entry(format!("{k:?}")).or_default();
            if entry.is_empty() {
                order.push(k);
            }
            entry.push(i);
        }
        let first_rows: Vec<usize> = order.iter().map(|k| groups[&format!("{k:?}")][0]).collect();
        let mut out = vec![key_col.take(&first_rows)];
        for (cname, agg) in aggs {
            let col = self.column(cname)?;
            let data: Vec<Option<f64>> = order
                .iter()
                .map(|k| agg.apply(col, &groups[&format!("{k:?}")]))
                .collect();
            let data = if *agg == Agg::Count {
                ColumnData::Int(data.into_iter().map(|x| x.map(|f| f as i64)).collect())
            } else {
                ColumnData::Float(data)
            };
            out.push(Column::new(format!("{cname}_{}", agg.name()), data));
        }
        DataFrame::new(out)
    }

    /// 数値列同士のピアソン相関(両方が非欠損の行のみ使用)。
    pub fn correlation(&self, a: &str, b: &str) -> Result<Option<f64>> {
        let (x, y) = self.paired(a, b)?;
        Ok(stats::correlation(&x, &y))
    }

    /// y = a + b x の単回帰(予測の最小単位)。
    pub fn linear_regression(&self, x: &str, y: &str) -> Result<Option<(f64, f64)>> {
        let (xs, ys) = self.paired(x, y)?;
        Ok(stats::linear_regression(&xs, &ys))
    }

    fn paired(&self, a: &str, b: &str) -> Result<(Vec<f64>, Vec<f64>)> {
        let (ca, cb) = (self.column(a)?, self.column(b)?);
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        for i in 0..self.height() {
            if let (Some(x), Some(y)) = (ca.get(i).as_f64(), cb.get(i).as_f64()) {
                xs.push(x);
                ys.push(y);
            }
        }
        Ok((xs, ys))
    }
}

/// 表形式で表示(ターミナル向け、全角文字は幅2として揃える)。
impl fmt::Display for DataFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn width(s: &str) -> usize {
            s.chars().map(|c| if c.is_ascii() { 1 } else { 2 }).sum()
        }
        fn cell(v: Value) -> String {
            match v {
                Value::Null => "null".into(),
                Value::Float(x) => format!("{x:.4}")
                    .trim_end_matches('0')
                    .trim_end_matches('.')
                    .to_string(),
                v => v.to_string(),
            }
        }
        let header: Vec<String> = self.columns.iter().map(|c| c.name.clone()).collect();
        let rows: Vec<Vec<String>> = (0..self.height())
            .map(|i| self.columns.iter().map(|c| cell(c.get(i))).collect())
            .collect();
        let widths: Vec<usize> = (0..self.width())
            .map(|j| {
                rows.iter()
                    .map(|r| width(&r[j]))
                    .chain([width(&header[j])])
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        let line = |f: &mut fmt::Formatter<'_>, cells: &[String]| -> fmt::Result {
            let parts: Vec<String> = cells
                .iter()
                .zip(&widths)
                .map(|(s, w)| format!("{s}{}", " ".repeat(w - width(s))))
                .collect();
            writeln!(f, "| {} |", parts.join(" | "))
        };
        line(f, &header)?;
        let sep: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
        writeln!(f, "|-{}-|", sep.join("-|-"))?;
        for r in &rows {
            line(f, r)?;
        }
        write!(f, "({} rows × {} columns)", self.height(), self.width())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::csv::read_csv_str;

    fn sample() -> DataFrame {
        read_csv_str("city,sales,qty\nTokyo,100,1\nOsaka,200,2\nTokyo,,3\nTokyo,300,4\nOsaka,50,\n")
            .unwrap()
    }

    #[test]
    fn filter_and_sort() {
        let df = sample();
        let f = df.filter_expr("sales>=100").unwrap();
        assert_eq!(f.height(), 3);
        let s = f.sort_by("sales", false).unwrap();
        assert_eq!(s.column("sales").unwrap().get(0), Value::Int(300));
        let t = df.filter_expr("city==Osaka").unwrap();
        assert_eq!(t.height(), 2);
        assert!(df.filter_expr("sales>abc").is_err());
    }

    #[test]
    fn cleansing() {
        let df = sample();
        assert_eq!(df.drop_nulls().height(), 3);
        let filled = df.fill_null("sales", &Fill::Mean).unwrap();
        assert_eq!(filled.column("sales").unwrap().get(2), Value::Float(162.5));
        let dup = read_csv_str("a,b\n1,x\n1,x\n2,x\n").unwrap();
        assert_eq!(dup.drop_duplicates().height(), 2);
    }

    #[test]
    fn group_by_aggs() {
        let g = sample()
            .group_by(
                "city",
                &[
                    ("sales", Agg::Sum),
                    ("sales", Agg::Count),
                    ("qty", Agg::Mean),
                ],
            )
            .unwrap();
        assert_eq!(g.height(), 2);
        assert_eq!(g.column("city").unwrap().get(0), Value::Str("Tokyo".into()));
        assert_eq!(g.column("sales_sum").unwrap().get(0), Value::Float(400.0));
        assert_eq!(g.column("sales_count").unwrap().get(0), Value::Int(2));
        assert_eq!(g.column("qty_mean").unwrap().get(1), Value::Float(2.0));
    }

    #[test]
    fn describe_and_regression() {
        let d = sample().describe();
        assert_eq!(d.height(), 3);
        assert_eq!(d.column("mean").unwrap().get(1), Value::Float(162.5));
        assert_eq!(d.column("mean").unwrap().get(0), Value::Null);
        let df = read_csv_str("x,y\n1,3\n2,5\n3,7\n").unwrap();
        let (a, b) = df.linear_regression("x", "y").unwrap().unwrap();
        assert!((a - 1.0).abs() < 1e-12 && (b - 2.0).abs() < 1e-12);
    }
}
