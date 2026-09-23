//! rrd: realdata.pro コマンドライン。
//!
//! 例:
//!   rrd describe data.csv
//!   rrd query data.csv --filter "sales>=100" --drop-nulls --sort sales:desc --head 10
//!   rrd groupby data.csv city sales:sum sales:mean
//!   rrd regress data.csv x y

use std::process::ExitCode;

use rrd_core::{csv, Agg, DataFrame, Fill, Result, Value};

const USAGE: &str = "\
使い方:
  rrd describe <file.csv>                       各列の要約統計
  rrd head <file.csv> [n]                       先頭n行(既定10)
  rrd query <file.csv> [オプション...]          クレンジング・抽出
      --filter <式>        例: \"sales>=100\" \"city==Tokyo\"(複数可)
      --select <a,b,...>   列の選択
      --drop-nulls         欠損を含む行を削除
      --dedup              重複行を削除
      --fill <列>:<mean|median|値>
      --sort <列>[:desc]
      --head <n>
      --csv                表ではなくCSVで出力
  rrd groupby <file.csv> <キー列> <列:集計>...  集計: count sum mean min max median std
  rrd corr <file.csv> <列A> <列B>               ピアソン相関
  rrd regress <file.csv> <x列> <y列>            単回帰 y = a + b x";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("エラー: {msg}\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

fn load(args: &[String]) -> std::result::Result<DataFrame, String> {
    let path = args.get(1).ok_or("CSVファイルを指定してください")?;
    csv::read_csv(path).map_err(|e| format!("{path}: {e}"))
}

fn run(args: &[String]) -> std::result::Result<(), String> {
    let cmd = args
        .first()
        .map(String::as_str)
        .ok_or("サブコマンドを指定してください")?;
    if matches!(cmd, "-h" | "--help" | "help") {
        println!("{USAGE}");
        return Ok(());
    }
    let df = load(args)?;
    match cmd {
        "describe" => println!("{}", df.describe()),
        "head" => {
            let n = args
                .get(2)
                .map_or(Ok(10), |s| s.parse())
                .map_err(|_| "行数が不正です")?;
            println!("{}", df.head(n));
        }
        "query" => query(df, &args[2..]).map_err(|e| e.to_string())?,
        "groupby" => {
            let key = args.get(2).ok_or("キー列を指定してください")?;
            let specs = args[3..]
                .iter()
                .map(|s| {
                    let (c, a) = s
                        .split_once(':')
                        .ok_or(format!("'{s}' は <列:集計> 形式ではありません"))?;
                    Ok((c, Agg::parse(a).map_err(|e| e.to_string())?))
                })
                .collect::<std::result::Result<Vec<(&str, Agg)>, String>>()?;
            println!("{}", df.group_by(key, &specs).map_err(|e| e.to_string())?);
        }
        "corr" | "regress" => {
            let (a, b) = match (args.get(2), args.get(3)) {
                (Some(a), Some(b)) => (a, b),
                _ => return Err("列を2つ指定してください".into()),
            };
            if cmd == "corr" {
                match df.correlation(a, b).map_err(|e| e.to_string())? {
                    Some(r) => println!("r({a}, {b}) = {r:.6}"),
                    None => println!("相関を計算できません(データ不足または分散0)"),
                }
            } else {
                match df.linear_regression(a, b).map_err(|e| e.to_string())? {
                    Some((i, s)) => println!("{b} = {i:.6} + {s:.6} * {a}"),
                    None => println!("回帰を計算できません(データ不足または分散0)"),
                }
            }
        }
        other => return Err(format!("未知のサブコマンド: {other}")),
    }
    Ok(())
}

fn query(mut df: DataFrame, opts: &[String]) -> Result<()> {
    let mut as_csv = false;
    let mut it = opts.iter();
    let need = |v: Option<&String>, o: &str| {
        v.cloned()
            .ok_or_else(|| rrd_core::Error::Invalid(format!("{o} に値がありません")))
    };
    while let Some(o) = it.next() {
        match o.as_str() {
            "--filter" => df = df.filter_expr(&need(it.next(), o)?)?,
            "--select" => {
                let s = need(it.next(), o)?;
                df = df.select(&s.split(',').map(str::trim).collect::<Vec<_>>())?;
            }
            "--drop-nulls" => df = df.drop_nulls(),
            "--dedup" => df = df.drop_duplicates(),
            "--fill" => {
                let s = need(it.next(), o)?;
                let (c, how) = s.split_once(':').ok_or_else(|| {
                    rrd_core::Error::Invalid("--fill は <列>:<方法> 形式です".into())
                })?;
                let how = match how {
                    "mean" => Fill::Mean,
                    "median" => Fill::Median,
                    v => Fill::Value(
                        v.parse::<i64>()
                            .map(Value::Int)
                            .unwrap_or_else(|_| Value::Str(v.into())),
                    ),
                };
                df = df.fill_null(c, &how)?;
            }
            "--sort" => {
                let s = need(it.next(), o)?;
                let (c, desc) = match s.split_once(':') {
                    Some((c, d)) => (c.to_string(), d.eq_ignore_ascii_case("desc")),
                    None => (s.clone(), false),
                };
                df = df.sort_by(&c, !desc)?;
            }
            "--head" => {
                let n = need(it.next(), o)?
                    .parse()
                    .map_err(|_| rrd_core::Error::Invalid("--head の値が不正です".into()))?;
                df = df.head(n);
            }
            "--csv" => as_csv = true,
            other => {
                return Err(rrd_core::Error::Invalid(format!(
                    "未知のオプション: {other}"
                )))
            }
        }
    }
    if as_csv {
        print!("{}", csv::to_csv_string(&df));
    } else {
        println!("{df}");
    }
    Ok(())
}
