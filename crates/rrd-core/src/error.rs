use std::fmt;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Parse(String),
    ColumnNotFound(String),
    Invalid(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "入出力エラー: {e}"),
            Error::Parse(m) => write!(f, "解析エラー: {m}"),
            Error::ColumnNotFound(c) => write!(f, "列が見つかりません: {c}"),
            Error::Invalid(m) => write!(f, "不正な操作: {m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
