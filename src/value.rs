use serde::{Deserialize, Serialize};
use std::fmt;

/// Compact wire values. Ordinary strings, including `=SUM(A1:A3)`, are text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Cell {
    Blank,
    Number(f64),
    Text(String),
    Bool(bool),
    Formula(Formula),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Formula {
    pub formula: String,
}

impl Cell {
    pub fn formula(source: impl Into<String>) -> Self {
        Self::Formula(Formula {
            formula: source.into(),
        })
    }
}

/// Calculated values retain the distinction between an error and ordinary text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Blank,
    Number(f64),
    Text(String),
    Bool(bool),
    Error(ErrorValue),
    Formula(Formula),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorValue {
    pub error: EngineError,
}

impl Value {
    pub(crate) fn from_cell(cell: &Cell) -> Self {
        match cell {
            Cell::Blank => Self::Blank,
            Cell::Number(value) => Self::Number(*value),
            Cell::Text(value) => Self::Text(value.clone()),
            Cell::Bool(value) => Self::Bool(*value),
            Cell::Formula(value) => Self::Formula(value.clone()),
        }
    }

    pub(crate) fn numeric(&self) -> Result<f64, EngineError> {
        match self {
            Self::Blank => Ok(0.0),
            Self::Number(value) => Ok(*value),
            Self::Bool(value) => Ok(if *value { 1.0 } else { 0.0 }),
            _ => Err(EngineError::new(
                ErrorCode::FormulaType,
                "Expected a number, boolean or blank; text is not coerced",
            )),
        }
    }

    pub(crate) fn truthy(&self) -> Result<bool, EngineError> {
        Ok(self.numeric()? != 0.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidInput,
    InvalidReference,
    FormulaSyntax,
    UnsupportedFunction,
    FormulaType,
    DivideByZero,
    Cycle,
    NonFinite,
    Limit,
    Version,
    Serialization,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineError {
    pub code: ErrorCode,
    pub message: String,
}

impl EngineError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub(crate) fn input(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidInput, message)
    }

    pub(crate) fn limit(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Limit, message)
    }

    pub(crate) fn reference(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidReference, message)
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for EngineError {}
