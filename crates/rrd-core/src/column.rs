//! 列(Column)の定義。各列は型付きの`Vec<Option<T>>`で保持する(None=欠損値)。

use std::fmt;

/// 列のデータ型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    Int,
    Float,
    Bool,
    Str,
}

impl fmt::Display for DataType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            DataType::Int => "int",
            DataType::Float => "float",
            DataType::Bool => "bool",
            DataType::Str => "str",
        };
        f.write_str(s)
    }
}

/// 列の実データ。
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnData {
    Int(Vec<Option<i64>>),
    Float(Vec<Option<f64>>),
    Bool(Vec<Option<bool>>),
    Str(Vec<Option<String>>),
}

/// 1セルの値(行単位アクセス・比較用)。
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
}

impl Value {
    /// 数値として解釈できれば f64 を返す。
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(v) => Some(*v as f64),
            Value::Float(v) => Some(*v),
            Value::Bool(v) => Some(if *v { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    /// 並べ替え・比較用の全順序比較(Nullは常に最小)。
    pub fn total_cmp(&self, other: &Value) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        match (self, other) {
            (Value::Null, Value::Null) => Ordering::Equal,
            (Value::Null, _) => Ordering::Less,
            (_, Value::Null) => Ordering::Greater,
            (Value::Str(a), Value::Str(b)) => a.cmp(b),
            (Value::Str(_), _) => Ordering::Greater,
            (_, Value::Str(_)) => Ordering::Less,
            (a, b) => a
                .as_f64()
                .unwrap_or(f64::NAN)
                .total_cmp(&b.as_f64().unwrap_or(f64::NAN)),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => Ok(()),
            Value::Int(v) => write!(f, "{v}"),
            Value::Float(v) => write!(f, "{v}"),
            Value::Bool(v) => write!(f, "{v}"),
            Value::Str(v) => f.write_str(v),
        }
    }
}

/// 名前付きの列。
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub name: String,
    pub data: ColumnData,
}

impl Column {
    pub fn new(name: impl Into<String>, data: ColumnData) -> Self {
        Self {
            name: name.into(),
            data,
        }
    }

    pub fn len(&self) -> usize {
        match &self.data {
            ColumnData::Int(v) => v.len(),
            ColumnData::Float(v) => v.len(),
            ColumnData::Bool(v) => v.len(),
            ColumnData::Str(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn dtype(&self) -> DataType {
        match &self.data {
            ColumnData::Int(_) => DataType::Int,
            ColumnData::Float(_) => DataType::Float,
            ColumnData::Bool(_) => DataType::Bool,
            ColumnData::Str(_) => DataType::Str,
        }
    }

    pub fn get(&self, i: usize) -> Value {
        match &self.data {
            ColumnData::Int(v) => v[i].map_or(Value::Null, Value::Int),
            ColumnData::Float(v) => v[i].map_or(Value::Null, Value::Float),
            ColumnData::Bool(v) => v[i].map_or(Value::Null, Value::Bool),
            ColumnData::Str(v) => v[i].clone().map_or(Value::Null, Value::Str),
        }
    }

    pub fn is_null(&self, i: usize) -> bool {
        match &self.data {
            ColumnData::Int(v) => v[i].is_none(),
            ColumnData::Float(v) => v[i].is_none(),
            ColumnData::Bool(v) => v[i].is_none(),
            ColumnData::Str(v) => v[i].is_none(),
        }
    }

    pub fn null_count(&self) -> usize {
        (0..self.len()).filter(|&i| self.is_null(i)).count()
    }

    /// 欠損を除いた数値のみを取り出す(数値列・bool列のみ。文字列列は空)。
    pub fn numeric_values(&self) -> Vec<f64> {
        match &self.data {
            ColumnData::Int(v) => v.iter().flatten().map(|x| *x as f64).collect(),
            ColumnData::Float(v) => v.iter().flatten().copied().collect(),
            ColumnData::Bool(v) => v
                .iter()
                .flatten()
                .map(|b| if *b { 1.0 } else { 0.0 })
                .collect(),
            ColumnData::Str(_) => Vec::new(),
        }
    }

    pub fn is_numeric(&self) -> bool {
        matches!(self.data, ColumnData::Int(_) | ColumnData::Float(_))
    }

    /// 指定した行番号だけを抜き出した新しい列を作る。
    pub fn take(&self, idx: &[usize]) -> Column {
        let data = match &self.data {
            ColumnData::Int(v) => ColumnData::Int(idx.iter().map(|&i| v[i]).collect()),
            ColumnData::Float(v) => ColumnData::Float(idx.iter().map(|&i| v[i]).collect()),
            ColumnData::Bool(v) => ColumnData::Bool(idx.iter().map(|&i| v[i]).collect()),
            ColumnData::Str(v) => ColumnData::Str(idx.iter().map(|&i| v[i].clone()).collect()),
        };
        Column::new(self.name.clone(), data)
    }
}
