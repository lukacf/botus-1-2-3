//! Botus 1-2-3: the spreadsheet that finally hired an agent to press F9.
//!
//! The engine is framework-independent. Optional adapters use their host's
//! durable document services and authenticated dispatch context.

pub mod engine;
mod formula;
mod grid;
mod value;

#[cfg(feature = "mobkit")]
pub mod native;

#[cfg(feature = "mcp")]
pub mod mcp;

pub use engine::{
    ApplyResult, Cell, ChangedRange, EngineError, ErrorCode, ErrorValue, Filter, FilterOp, Formula,
    Limits, NamedRange, Operation, ReadRequest, ReadResult, ReadView, Sheet, SheetDescription,
    SortKey, Value, Workbook, WorkbookDescription,
};
