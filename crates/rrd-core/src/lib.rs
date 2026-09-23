//! realdata.pro コアエンジン。
//!
//! インメモリ列指向データフレームで、SAS Viya的な分析ライフサイクル
//! (読込 → クレンジング → 探索的分析 → 集計 → 予測)の最小構成を提供する。
//! 外部クレート依存なし。

pub mod column;
pub mod csv;
pub mod error;
pub mod frame;
pub mod stats;

pub use column::{Column, ColumnData, DataType, Value};
pub use error::{Error, Result};
pub use frame::{Agg, CmpOp, DataFrame, Fill};
