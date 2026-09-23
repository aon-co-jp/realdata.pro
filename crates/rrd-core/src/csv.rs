//! CSV読み書き(RFC 4180準拠のクォート対応、依存クレートなし)と列型の自動推論。

use crate::column::{Column, ColumnData};
use crate::error::{Error, Result};
use crate::frame::DataFrame;

/// 欠損値とみなす文字列。
fn is_null_token(s: &str) -> bool {
    matches!(s.trim(), "" | "NA" | "N/A" | "null" | "NULL" | "NaN")
}

/// CSVテキストを行×フィールドに分解する(クォート内の改行・カンマ・""エスケープ対応)。
fn parse_records(text: &str) -> Result<Vec<Vec<String>>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();
    let mut line = 1usize;

    while let Some(c) = chars.next() {
        if in_quotes {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => in_quotes = false,
                '\n' => {
                    line += 1;
                    field.push(c);
                }
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => in_quotes = true,
            ',' => record.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                line += 1;
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
            }
            _ => field.push(c),
        }
    }
    if in_quotes {
        return Err(Error::Parse(format!(
            "{line}行目: クォートが閉じていません"
        )));
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    // 完全な空行は除外
    records.retain(|r| !(r.len() == 1 && r[0].is_empty()));
    Ok(records)
}

/// 文字列の列から型を推論して ColumnData を作る(int → float → bool → str の順に試す)。
fn infer_column(raw: Vec<Option<String>>) -> ColumnData {
    let non_null = || raw.iter().flatten();
    if non_null().all(|s| s.trim().parse::<i64>().is_ok()) {
        return ColumnData::Int(
            raw.iter()
                .map(|o| o.as_ref().map(|s| s.trim().parse().unwrap()))
                .collect(),
        );
    }
    if non_null().all(|s| s.trim().parse::<f64>().is_ok()) {
        return ColumnData::Float(
            raw.iter()
                .map(|o| o.as_ref().map(|s| s.trim().parse().unwrap()))
                .collect(),
        );
    }
    let as_bool = |s: &str| match s.trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    };
    if non_null().all(|s| as_bool(s).is_some()) {
        return ColumnData::Bool(raw.iter().map(|o| o.as_deref().and_then(as_bool)).collect());
    }
    ColumnData::Str(raw)
}

/// CSVテキスト(1行目はヘッダ)から DataFrame を作る。
pub fn read_csv_str(text: &str) -> Result<DataFrame> {
    let mut records = parse_records(text)?.into_iter();
    let header = records
        .next()
        .ok_or_else(|| Error::Parse("ヘッダ行がありません".into()))?;
    let ncol = header.len();
    let mut raw: Vec<Vec<Option<String>>> = vec![Vec::new(); ncol];
    for (i, rec) in records.enumerate() {
        if rec.len() != ncol {
            return Err(Error::Parse(format!(
                "データ{}行目: 列数{}がヘッダの列数{}と一致しません",
                i + 1,
                rec.len(),
                ncol
            )));
        }
        for (j, v) in rec.into_iter().enumerate() {
            raw[j].push(if is_null_token(&v) { None } else { Some(v) });
        }
    }
    let columns = header
        .into_iter()
        .zip(raw)
        .map(|(name, vals)| Column::new(name.trim(), infer_column(vals)))
        .collect();
    DataFrame::new(columns)
}

/// ファイルから読み込む。
pub fn read_csv(path: impl AsRef<std::path::Path>) -> Result<DataFrame> {
    read_csv_str(&std::fs::read_to_string(path)?)
}

fn escape(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// DataFrame をCSVテキストに変換する(欠損は空欄)。
pub fn to_csv_string(df: &DataFrame) -> String {
    let mut out = String::new();
    let names: Vec<String> = df.columns().iter().map(|c| escape(&c.name)).collect();
    out.push_str(&names.join(","));
    out.push('\n');
    for i in 0..df.height() {
        let row: Vec<String> = df
            .columns()
            .iter()
            .map(|c| escape(&c.get(i).to_string()))
            .collect();
        out.push_str(&row.join(","));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::column::DataType;

    #[test]
    fn infers_types_and_nulls() {
        let df =
            read_csv_str("a,b,c,d\n1,1.5,true,x\n,2,false,\"y,z\"\n3,NA,,\"he said \"\"hi\"\"\"\n")
                .unwrap();
        assert_eq!(df.height(), 3);
        assert_eq!(df.column("a").unwrap().dtype(), DataType::Int);
        assert_eq!(df.column("b").unwrap().dtype(), DataType::Float);
        assert_eq!(df.column("c").unwrap().dtype(), DataType::Bool);
        assert_eq!(df.column("d").unwrap().dtype(), DataType::Str);
        assert_eq!(df.column("a").unwrap().null_count(), 1);
        assert_eq!(df.column("d").unwrap().get(2).to_string(), "he said \"hi\"");
    }

    #[test]
    fn roundtrip() {
        let src = "name,v\n\"a,b\",1\nc,\n";
        let df = read_csv_str(src).unwrap();
        assert_eq!(to_csv_string(&df), src);
    }

    #[test]
    fn rejects_ragged_rows() {
        assert!(read_csv_str("a,b\n1\n").is_err());
        assert!(read_csv_str("a\n\"open\n").is_err());
    }
}
