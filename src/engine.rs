//! Sparse, bounded spreadsheets. The cells have escaped the beige computer.

use crate::formula::{self, Context, Expr, Reference};
use crate::grid::{Address, GridRange, column_name, parse_column};
pub use crate::value::{Cell, EngineError, ErrorCode, ErrorValue, Formula, Value};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

pub const SCHEMA_VERSION: u32 = 1;

/// Host-selected resource limits apply across each complete batch or read.
#[derive(Clone, Debug)]
pub struct Limits {
    pub max_sheets: usize,
    pub max_rows: u32,
    pub max_columns: u32,
    pub max_cells: usize,
    pub max_cell_text_bytes: usize,
    pub max_total_text_bytes: usize,
    pub max_document_bytes: usize,
    pub max_formula_bytes: usize,
    pub max_formula_nodes: usize,
    pub max_batch_operations: usize,
    pub max_operation_cells: usize,
    pub max_read_rows: usize,
    pub max_read_cells: usize,
    pub max_scan_rows: usize,
    pub max_evaluation_steps: usize,
    pub max_dependency_depth: usize,
    pub max_response_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_sheets: 32,
            max_rows: 100_000,
            max_columns: 1024,
            max_cells: 100_000,
            max_cell_text_bytes: 1024 * 1024,
            max_total_text_bytes: 8 * 1024 * 1024,
            max_document_bytes: 16 * 1024 * 1024,
            max_formula_bytes: 16 * 1024,
            max_formula_nodes: 512,
            max_batch_operations: 128,
            max_operation_cells: 100_000,
            max_read_rows: 1000,
            max_read_cells: 10_000,
            max_scan_rows: 10_000,
            max_evaluation_steps: 250_000,
            max_dependency_depth: 64,
            max_response_bytes: 256 * 1024,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workbook {
    pub schema_version: u32,
    #[serde(deserialize_with = "unique_map")]
    pub sheets: BTreeMap<String, Sheet>,
    #[serde(deserialize_with = "unique_map")]
    pub names: BTreeMap<String, NamedRange>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sheet {
    #[serde(deserialize_with = "unique_map")]
    pub cells: BTreeMap<String, Cell>,
}

// A damaged floppy was never permission to quietly discard the first record.
fn unique_map<'de, D, V>(deserializer: D) -> Result<BTreeMap<String, V>, D::Error>
where
    D: serde::Deserializer<'de>,
    V: Deserialize<'de>,
{
    struct Unique<V>(std::marker::PhantomData<V>);
    impl<'de, V: Deserialize<'de>> serde::de::Visitor<'de> for Unique<V> {
        type Value = BTreeMap<String, V>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a map with unique keys")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> Result<Self::Value, M::Error> {
            let mut result = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, V>()? {
                if result.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom("duplicate stored map key"));
                }
            }
            Ok(result)
        }
    }
    deserializer.deserialize_map(Unique(std::marker::PhantomData))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamedRange {
    pub sheet: String,
    pub range: String,
}

fn default_sheet() -> String {
    "Sheet1".to_owned()
}
fn default_limit() -> usize {
    100
}

/// All writes are explicit. A string that resembles a formula remains text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    CreateSheet {
        name: String,
    },
    Put {
        #[serde(default = "default_sheet")]
        sheet: String,
        at: String,
        values: Vec<Vec<Cell>>,
    },
    Formula {
        #[serde(default = "default_sheet")]
        sheet: String,
        at: String,
        formula: String,
    },
    Clear {
        #[serde(default = "default_sheet")]
        sheet: String,
        range: String,
    },
    Copy {
        #[serde(default = "default_sheet")]
        sheet: String,
        range: String,
        to: String,
        #[serde(default)]
        target_sheet: Option<String>,
    },
    Fill {
        #[serde(default = "default_sheet")]
        sheet: String,
        range: String,
        to: String,
    },
    Sort {
        #[serde(default = "default_sheet")]
        sheet: String,
        range: String,
        keys: Vec<SortKey>,
        #[serde(default)]
        header: bool,
    },
    Name {
        name: String,
        #[serde(default = "default_sheet")]
        sheet: String,
        range: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SortKey {
    pub column: String,
    #[serde(default)]
    pub descending: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Filter {
    pub column: String,
    pub op: FilterOp,
    pub value: Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Contains,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadView {
    #[default]
    Values,
    Formulas,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadRequest {
    #[serde(default = "default_sheet")]
    pub sheet: String,
    #[serde(default)]
    pub range: Option<String>,
    #[serde(default)]
    pub columns: Option<Vec<String>>,
    #[serde(default)]
    pub view: ReadView,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub filter: Option<Filter>,
}

impl Default for ReadRequest {
    fn default() -> Self {
        Self {
            sheet: default_sheet(),
            range: None,
            columns: None,
            view: ReadView::Values,
            offset: 0,
            limit: default_limit(),
            filter: None,
        }
    }
}

/// Rows are compact matrices. Row numbers and column names identify gaps.
/// Pagination offsets count physical rows in the selected range, not matches.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReadResult {
    pub sheet: String,
    pub range: Option<String>,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub row_numbers: Vec<u32>,
    pub next_offset: Option<usize>,
    pub scanned_rows: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChangedRange {
    pub sheet: String,
    pub range: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApplyResult {
    pub operations: usize,
    pub changed_cells: usize,
    pub changed_ranges: Vec<ChangedRange>,
    pub sheets: usize,
    pub total_cells: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkbookDescription {
    pub schema_version: u32,
    pub sheets: Vec<SheetDescription>,
    pub names: BTreeMap<String, NamedRange>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SheetDescription {
    pub name: String,
    pub occupied_range: Option<String>,
    pub cells: usize,
    pub formulas: usize,
}

impl Default for Workbook {
    fn default() -> Self {
        Self::new()
    }
}

impl Workbook {
    pub fn new() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            sheets: BTreeMap::from([(default_sheet(), Sheet::default())]),
            names: BTreeMap::new(),
        }
    }

    pub fn from_json(bytes: &[u8], limits: &Limits) -> Result<Self, EngineError> {
        if bytes.len() > limits.max_document_bytes {
            return Err(EngineError::limit(
                "Stored workbook exceeds document byte limit",
            ));
        }
        let workbook: Self = serde_json::from_slice(bytes).map_err(|error| {
            EngineError::new(
                ErrorCode::Serialization,
                format!("Invalid workbook JSON: {error}"),
            )
        })?;
        workbook.validate(limits)?;
        Ok(workbook)
    }

    pub fn to_json(&self, limits: &Limits) -> Result<Vec<u8>, EngineError> {
        self.validate(limits)?;
        let bytes = serde_json::to_vec(self)
            .map_err(|error| EngineError::new(ErrorCode::Serialization, error.to_string()))?;
        if bytes.len() > limits.max_document_bytes {
            return Err(EngineError::limit("Workbook exceeds document byte limit"));
        }
        Ok(bytes)
    }

    pub fn describe(&self, limits: &Limits) -> Result<WorkbookDescription, EngineError> {
        self.validate(limits)?;
        let description = WorkbookDescription {
            schema_version: self.schema_version,
            sheets: self
                .sheets
                .iter()
                .map(|(name, sheet)| {
                    Ok(SheetDescription {
                        name: name.clone(),
                        occupied_range: bounds(sheet)?.map(|range| range.to_string()),
                        cells: sheet.cells.len(),
                        formulas: sheet
                            .cells
                            .values()
                            .filter(|cell| matches!(cell, Cell::Formula(_)))
                            .count(),
                    })
                })
                .collect::<Result<Vec<_>, EngineError>>()?,
            names: self.names.clone(),
        };
        check_response_size(&description, limits)?;
        Ok(description)
    }

    /// Validation checks stored structure and syntax, without persisting caches.
    pub fn validate(&self, limits: &Limits) -> Result<(), EngineError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(EngineError::new(
                ErrorCode::Version,
                "Unsupported workbook schema version",
            ));
        }
        if self.sheets.is_empty() || self.sheets.len() > limits.max_sheets {
            return Err(EngineError::limit("Workbook sheet count is outside limits"));
        }
        let mut names = BTreeSet::new();
        let mut cell_count = 0_usize;
        let mut text_bytes = 0_usize;
        for (name, sheet) in &self.sheets {
            validate_sheet_name(name)?;
            if !names.insert(name.to_lowercase()) {
                return Err(EngineError::input(
                    "Sheet names must be unique ignoring case",
                ));
            }
            cell_count = cell_count
                .checked_add(sheet.cells.len())
                .ok_or_else(|| EngineError::limit("Cell count overflow"))?;
            if cell_count > limits.max_cells {
                return Err(EngineError::limit("Workbook cell count exceeds limit"));
            }
            for (address, cell) in &sheet.cells {
                let parsed = parse_address(address, limits)?;
                if parsed.to_string() != *address {
                    return Err(EngineError::input(
                        "Stored cell addresses must be canonical uppercase",
                    ));
                }
                if matches!(cell, Cell::Blank) {
                    return Err(EngineError::input(
                        "Blank cells must be omitted from sparse storage",
                    ));
                }
                validate_cell(cell, limits)?;
                let bytes = match cell {
                    Cell::Text(value) => value.len(),
                    Cell::Formula(value) => value.formula.len(),
                    _ => 0,
                };
                text_bytes = text_bytes
                    .checked_add(bytes)
                    .ok_or_else(|| EngineError::limit("Workbook text byte count overflow"))?;
                if text_bytes > limits.max_total_text_bytes {
                    return Err(EngineError::limit("Workbook text exceeds total byte limit"));
                }
                if let Cell::Formula(formula) = cell {
                    let expr = formula::parse(&formula.formula, limits.max_formula_nodes)?;
                    formula::visit_references(&expr, &mut |reference| {
                        check_address(reference.address, limits)?;
                        if let Some(sheet) = &reference.sheet {
                            self.sheet_key(sheet)?;
                        }
                        Ok(())
                    })?;
                }
            }
        }
        if self.names.len() > limits.max_cells {
            return Err(EngineError::limit("Named range count exceeds limit"));
        }
        for (name, range) in &self.names {
            validate_name(name)?;
            if name.to_uppercase() != *name {
                return Err(EngineError::input(
                    "Stored range names must be canonical uppercase",
                ));
            }
            self.sheet_key(&range.sheet)?;
            let parsed = parse_range(&range.range, limits)?;
            if parsed.to_string() != range.range {
                return Err(EngineError::input("Stored named ranges must be canonical"));
            }
        }
        Ok(())
    }

    /// A batch either commits every operation and valid calculation, or none.
    pub fn apply(
        &mut self,
        operations: &[Operation],
        limits: &Limits,
    ) -> Result<ApplyResult, EngineError> {
        self.validate(limits)?;
        if operations.is_empty() || operations.len() > limits.max_batch_operations {
            return Err(EngineError::limit(
                "Batch operation count is outside limits",
            ));
        }
        let mut candidate = self.clone();
        let mut operation_budget = MutationBudget {
            work: limits.max_operation_cells,
            cells: candidate
                .sheets
                .values()
                .map(|sheet| sheet.cells.len())
                .sum(),
            text_bytes: candidate
                .sheets
                .values()
                .flat_map(|sheet| sheet.cells.values())
                .map(cell_text_bytes)
                .sum(),
        };
        for operation in operations {
            candidate.apply_one(operation, limits, &mut operation_budget)?;
        }
        candidate.validate(limits)?;
        let mut evaluator = Evaluator::new(&candidate, limits);
        for (sheet, data) in &candidate.sheets {
            for (address, cell) in &data.cells {
                if matches!(cell, Cell::Formula(_)) {
                    evaluator.cell_at(
                        sheet,
                        Address::parse(address).map_err(EngineError::reference)?,
                    )?;
                }
            }
        }
        candidate.to_json(limits)?;
        let (changed_cells, changed_ranges) = changes(self, &candidate)?;
        let result = ApplyResult {
            operations: operations.len(),
            changed_cells,
            changed_ranges,
            sheets: candidate.sheets.len(),
            total_cells: candidate
                .sheets
                .values()
                .map(|sheet| sheet.cells.len())
                .sum(),
        };
        check_response_size(&result, limits)?;
        *self = candidate;
        Ok(result)
    }

    pub fn read(&self, request: &ReadRequest, limits: &Limits) -> Result<ReadResult, EngineError> {
        self.validate(limits)?;
        if request.limit == 0 || request.limit > limits.max_read_rows {
            return Err(EngineError::limit("Read row limit is outside limits"));
        }
        let sheet_name = self.sheet_key(&request.sheet)?.to_owned();
        let sheet = self
            .sheets
            .get(&sheet_name)
            .ok_or_else(|| EngineError::reference("Unknown sheet"))?;
        let range = match &request.range {
            Some(range) => Some(parse_range(range, limits)?),
            None => bounds(sheet)?,
        };
        let mut result = ReadResult {
            sheet: sheet_name.clone(),
            range: range.map(|range| range.to_string()),
            columns: vec![],
            rows: vec![],
            row_numbers: vec![],
            next_offset: None,
            scanned_rows: 0,
        };
        let Some(range) = range else {
            check_response_size(&result, limits)?;
            return Ok(result);
        };
        let columns = select_columns(request.columns.as_deref(), range, limits)?;
        result.columns = columns.iter().map(|column| column_name(*column)).collect();
        let page_rows = request.limit.min(limits.max_read_cells / columns.len());
        if page_rows == 0 {
            return Err(EngineError::limit(
                "Selected columns exceed read cell budget",
            ));
        }
        let filter_column = request
            .filter
            .as_ref()
            .map(|filter| {
                validate_filter(filter, limits)?;
                let column = parse_column(&filter.column).map_err(EngineError::input)?;
                if column < range.start.column || column > range.end.column {
                    return Err(EngineError::input(
                        "Filter column must be in the selected range",
                    ));
                }
                Ok(column)
            })
            .transpose()?;
        let total_rows = usize::try_from(range.height())
            .map_err(|_| EngineError::limit("Range is too large"))?;
        if request.offset > total_rows {
            return Err(EngineError::input("Read offset exceeds range height"));
        }
        let mut offset = request.offset;
        let mut evaluator = Evaluator::new(self, limits);
        let mut value_bytes = 0_usize;
        while offset < total_rows
            && result.rows.len() < page_rows
            && result.scanned_rows < limits.max_scan_rows
        {
            let row = range
                .start
                .row
                .checked_add(
                    u32::try_from(offset)
                        .map_err(|_| EngineError::limit("Read offset is too large"))?,
                )
                .ok_or_else(|| EngineError::limit("Read row overflow"))?;
            offset += 1;
            result.scanned_rows += 1;
            if let (Some(filter), Some(column)) = (&request.filter, filter_column) {
                let value = evaluator.cell_at(&sheet_name, Address { column, row })?;
                if !matches_filter(&value, filter)? {
                    continue;
                }
            }
            let mut values = Vec::with_capacity(columns.len());
            for column in &columns {
                let address = Address {
                    column: *column,
                    row,
                };
                let value = if request.view == ReadView::Formulas {
                    evaluator.charge()?;
                    sheet
                        .cells
                        .get(&address.to_string())
                        .map(Value::from_cell)
                        .unwrap_or(Value::Blank)
                } else {
                    match evaluator.cell_at(&sheet_name, address) {
                        Ok(value) => value,
                        Err(error) if error.code == ErrorCode::Limit => return Err(error),
                        Err(error) => Value::Error(ErrorValue { error }),
                    }
                };
                let serialized = serde_json::to_vec(&value).map_err(|error| {
                    EngineError::new(ErrorCode::Serialization, error.to_string())
                })?;
                value_bytes = value_bytes
                    .checked_add(serialized.len())
                    .ok_or_else(|| EngineError::limit("Result byte count overflow"))?;
                if value_bytes > limits.max_response_bytes {
                    return Err(EngineError::limit(
                        "Result exceeds serialized byte limit; select fewer rows or columns",
                    ));
                }
                values.push(value);
            }
            result.rows.push(values);
            result.row_numbers.push(row + 1);
            // Check as pages grow, so long text cannot multiply an output limit.
            check_response_size(&result, limits)?;
        }
        if offset < total_rows {
            result.next_offset = Some(offset);
        }
        check_response_size(&result, limits)?;
        Ok(result)
    }

    fn sheet_key(&self, name: &str) -> Result<&str, EngineError> {
        self.sheets
            .keys()
            .find(|key| key.to_lowercase() == name.to_lowercase())
            .map(String::as_str)
            .ok_or_else(|| EngineError::reference("Unknown sheet"))
    }

    fn set_cell(
        &mut self,
        sheet: &str,
        address: Address,
        cell: Cell,
        limits: &Limits,
        budget: &mut MutationBudget,
    ) -> Result<(), EngineError> {
        let cells = &mut self
            .sheets
            .get_mut(sheet)
            .ok_or_else(|| EngineError::reference("Unknown sheet"))?
            .cells;
        let key = address.to_string();
        let previous = cells.get(&key);
        let next_count = budget
            .cells
            .saturating_sub(usize::from(previous.is_some()))
            .checked_add(usize::from(cell != Cell::Blank))
            .ok_or_else(|| EngineError::limit("Cell count overflow"))?;
        let next_bytes = budget
            .text_bytes
            .saturating_sub(previous.map(cell_text_bytes).unwrap_or(0))
            .checked_add(cell_text_bytes(&cell))
            .ok_or_else(|| EngineError::limit("Text byte count overflow"))?;
        if next_count > limits.max_cells || next_bytes > limits.max_total_text_bytes {
            return Err(EngineError::limit(
                "Mutation exceeds workbook cell or text byte limit",
            ));
        }
        budget.cells = next_count;
        budget.text_bytes = next_bytes;
        if cell == Cell::Blank {
            cells.remove(&key);
        } else {
            cells.insert(key, cell);
        }
        Ok(())
    }

    fn apply_one(
        &mut self,
        operation: &Operation,
        limits: &Limits,
        budget: &mut MutationBudget,
    ) -> Result<(), EngineError> {
        match operation {
            Operation::CreateSheet { name } => {
                validate_sheet_name(name)?;
                if self.sheet_key(name).is_ok() {
                    return Err(EngineError::input("Sheet already exists"));
                }
                if self.sheets.len() >= limits.max_sheets {
                    return Err(EngineError::limit("Sheet limit exceeded"));
                }
                self.sheets.insert(name.clone(), Sheet::default());
            }
            Operation::Put { sheet, at, values } => {
                let sheet = self.sheet_key(sheet)?.to_owned();
                let start = parse_address(at, limits)?;
                let width = values.first().map(Vec::len).unwrap_or(0);
                if width == 0 || values.iter().any(|row| row.len() != width) {
                    return Err(EngineError::input(
                        "Put requires a nonempty rectangular matrix",
                    ));
                }
                spend(
                    &mut budget.work,
                    values
                        .len()
                        .checked_mul(width)
                        .ok_or_else(|| EngineError::limit("Matrix size overflow"))?,
                )?;
                for (row, values) in values.iter().enumerate() {
                    for (column, value) in values.iter().enumerate() {
                        let address = start
                            .shifted(
                                i64::try_from(column)
                                    .map_err(|_| EngineError::limit("Matrix is too wide"))?,
                                i64::try_from(row)
                                    .map_err(|_| EngineError::limit("Matrix is too tall"))?,
                            )
                            .map_err(EngineError::reference)?;
                        check_address(address, limits)?;
                        validate_cell(value, limits)?;
                        self.set_cell(&sheet, address, value.clone(), limits, budget)?;
                    }
                }
            }
            Operation::Formula { sheet, at, formula } => {
                let sheet = self.sheet_key(sheet)?.to_owned();
                let address = parse_address(at, limits)?;
                let cell = Cell::formula(formula);
                validate_cell(&cell, limits)?;
                spend(&mut budget.work, 1)?;
                self.set_cell(&sheet, address, cell, limits, budget)?;
            }
            Operation::Clear { sheet, range } => {
                let sheet = self.sheet_key(sheet)?.to_owned();
                let range = parse_range(range, limits)?;
                let cells = &self
                    .sheets
                    .get(&sheet)
                    .ok_or_else(|| EngineError::reference("Unknown sheet"))?
                    .cells;
                spend(&mut budget.work, cells.len())?;
                let addresses = cells
                    .keys()
                    .filter_map(|address| {
                        Address::parse(address)
                            .ok()
                            .filter(|address| range.contains(*address))
                    })
                    .collect::<Vec<_>>();
                for address in addresses {
                    self.set_cell(&sheet, address, Cell::Blank, limits, budget)?;
                }
            }
            Operation::Copy {
                sheet,
                range,
                to,
                target_sheet,
            } => {
                let source_sheet = self.sheet_key(sheet)?.to_owned();
                let target_sheet = self
                    .sheet_key(target_sheet.as_deref().unwrap_or(sheet))?
                    .to_owned();
                let source = parse_range(range, limits)?;
                let destination = parse_address(to, limits)?;
                let end = destination
                    .shifted(
                        i64::try_from(source.width() - 1)
                            .map_err(|_| EngineError::limit("Copy range too wide"))?,
                        i64::try_from(source.height() - 1)
                            .map_err(|_| EngineError::limit("Copy range too tall"))?,
                    )
                    .map_err(EngineError::reference)?;
                check_address(end, limits)?;
                self.copy_range(
                    &source_sheet,
                    source,
                    &target_sheet,
                    GridRange {
                        start: destination,
                        end,
                    },
                    false,
                    limits,
                    budget,
                )?;
            }
            Operation::Fill { sheet, range, to } => {
                let sheet = self.sheet_key(sheet)?.to_owned();
                let source = parse_range(range, limits)?;
                let destination = parse_range(to, limits)?;
                if destination.width() % source.width() != 0
                    || destination.height() % source.height() != 0
                {
                    return Err(EngineError::input(
                        "Fill destination must be a whole multiple of the source rectangle",
                    ));
                }
                self.copy_range(&sheet, source, &sheet, destination, true, limits, budget)?;
            }
            Operation::Sort {
                sheet,
                range,
                keys,
                header,
            } => self.sort_rows(sheet, range, keys, *header, limits, budget)?,
            Operation::Name { name, sheet, range } => {
                let name = name.to_uppercase();
                validate_name(&name)?;
                let sheet = self.sheet_key(sheet)?.to_owned();
                let range = parse_range(range, limits)?.to_string();
                self.names.insert(name, NamedRange { sheet, range });
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn copy_range(
        &mut self,
        source_sheet: &str,
        source: GridRange,
        target_sheet: &str,
        target: GridRange,
        tiled: bool,
        limits: &Limits,
        budget: &mut MutationBudget,
    ) -> Result<(), EngineError> {
        let area = usize::try_from(
            target
                .area()
                .ok_or_else(|| EngineError::limit("Copy area overflow"))?,
        )
        .map_err(|_| EngineError::limit("Copy area too large"))?;
        spend(&mut budget.work, area)?;
        // Snapshot just the bounded source, so overlapping copies are stable.
        let original = self
            .sheets
            .get(source_sheet)
            .ok_or_else(|| EngineError::reference("Unknown source sheet"))?
            .cells
            .iter()
            .filter_map(|(address, cell)| {
                Address::parse(address)
                    .ok()
                    .filter(|address| source.contains(*address))
                    .map(|address| (address, cell.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        let previous_targets = self
            .sheets
            .get(target_sheet)
            .ok_or_else(|| EngineError::reference("Unknown target sheet"))?
            .cells
            .keys()
            .filter_map(|address| {
                Address::parse(address)
                    .ok()
                    .filter(|address| target.contains(*address))
            })
            .collect::<Vec<_>>();
        // Release destination accounting before writes. In an overlap, a later
        // blank must not make an otherwise valid final result exceed limits.
        for address in previous_targets {
            self.set_cell(target_sheet, address, Cell::Blank, limits, budget)?;
        }
        for row in target.start.row..=target.end.row {
            for column in target.start.column..=target.end.column {
                let row_offset = u64::from(row - target.start.row);
                let col_offset = u64::from(column - target.start.column);
                let source_address = Address {
                    row: source.start.row
                        + u32::try_from(if tiled {
                            row_offset % source.height()
                        } else {
                            row_offset
                        })
                        .map_err(|_| EngineError::limit("Copy row overflow"))?,
                    column: source.start.column
                        + u32::try_from(if tiled {
                            col_offset % source.width()
                        } else {
                            col_offset
                        })
                        .map_err(|_| EngineError::limit("Copy column overflow"))?,
                };
                let cell = match original
                    .get(&source_address)
                    .cloned()
                    .unwrap_or(Cell::Blank)
                {
                    Cell::Formula(value) => Cell::formula(formula::shift(
                        &value.formula,
                        i64::from(column) - i64::from(source_address.column),
                        i64::from(row) - i64::from(source_address.row),
                        limits.max_columns,
                        limits.max_rows,
                        limits.max_formula_nodes,
                    )?),
                    value => value,
                };
                self.set_cell(target_sheet, Address { column, row }, cell, limits, budget)?;
            }
        }
        Ok(())
    }

    fn sort_rows(
        &mut self,
        sheet: &str,
        range: &str,
        keys: &[SortKey],
        header: bool,
        limits: &Limits,
        budget: &mut MutationBudget,
    ) -> Result<(), EngineError> {
        let sheet = self.sheet_key(sheet)?.to_owned();
        let range = parse_range(range, limits)?;
        if keys.is_empty() || keys.len() > 16 {
            return Err(EngineError::input("Sort requires one to sixteen keys"));
        }
        let keys = keys
            .iter()
            .map(|key| {
                let column = parse_column(&key.column).map_err(EngineError::input)?;
                if column < range.start.column || column > range.end.column {
                    return Err(EngineError::input(
                        "Sort key must be inside the selected range",
                    ));
                }
                Ok(((column - range.start.column) as usize, key.descending))
            })
            .collect::<Result<Vec<_>, EngineError>>()?;
        let area = usize::try_from(
            range
                .area()
                .ok_or_else(|| EngineError::limit("Sort area overflow"))?,
        )
        .map_err(|_| EngineError::limit("Sort area too large"))?;
        spend(&mut budget.work, area)?;
        let first_row = u64::from(range.start.row) + u64::from(header);
        let data = self
            .sheets
            .get(&sheet)
            .ok_or_else(|| EngineError::reference("Unknown sheet"))?;
        let mut rows = Vec::new();
        for row in first_row..=u64::from(range.end.row) {
            let row = u32::try_from(row).map_err(|_| EngineError::limit("Sort row overflow"))?;
            let mut cells = Vec::new();
            for column in range.start.column..=range.end.column {
                let cell = data
                    .cells
                    .get(&Address { column, row }.to_string())
                    .cloned()
                    .unwrap_or(Cell::Blank);
                if matches!(cell, Cell::Formula(_)) {
                    return Err(EngineError::input(
                        "Sorting a range containing formulas is unsupported; sort literal records before adding formulas",
                    ));
                }
                cells.push(cell);
            }
            rows.push(cells);
        }
        rows.sort_by(|left, right| {
            for (index, descending) in &keys {
                let ordering = compare_cells(&left[*index], &right[*index]);
                if ordering != Ordering::Equal {
                    return if *descending {
                        ordering.reverse()
                    } else {
                        ordering
                    };
                }
            }
            Ordering::Equal
        });
        // Remove the old records first, so a permutation cannot temporarily
        // double-count the same large string against the document budget.
        for row in first_row..=u64::from(range.end.row) {
            for column in range.start.column..=range.end.column {
                self.set_cell(
                    &sheet,
                    Address {
                        column,
                        row: row as u32,
                    },
                    Cell::Blank,
                    limits,
                    budget,
                )?;
            }
        }
        for (offset, values) in rows.into_iter().enumerate() {
            let row = u32::try_from(first_row + offset as u64)
                .map_err(|_| EngineError::limit("Sort row overflow"))?;
            for (offset, cell) in values.into_iter().enumerate() {
                self.set_cell(
                    &sheet,
                    Address {
                        column: range.start.column + offset as u32,
                        row,
                    },
                    cell,
                    limits,
                    budget,
                )?;
            }
        }
        Ok(())
    }
}

fn validate_sheet_name(name: &str) -> Result<(), EngineError> {
    if name.is_empty()
        || name.len() > 128
        || name.trim() != name
        || name
            .chars()
            .any(|c| c.is_control() || "[]:*?/\\!".contains(c))
    {
        return Err(EngineError::input(
            "Sheet name is empty, oversized or contains unsupported characters",
        ));
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<(), EngineError> {
    let first = name.chars().next();
    if name.is_empty()
        || name.len() > 128
        || !first.is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        || Address::parse(name).is_ok()
        || matches!(name, "TRUE" | "FALSE")
    {
        return Err(EngineError::input(
            "Range name must be an identifier, not a cell address or boolean",
        ));
    }
    Ok(())
}

fn validate_cell(cell: &Cell, limits: &Limits) -> Result<(), EngineError> {
    match cell {
        Cell::Number(value) if !value.is_finite() => {
            return Err(EngineError::new(
                ErrorCode::NonFinite,
                "Cell number must be finite",
            ));
        }
        Cell::Text(value) if value.len() > limits.max_cell_text_bytes => {
            return Err(EngineError::limit("Cell text exceeds byte limit"));
        }
        Cell::Formula(value) => {
            if value.formula.len() > limits.max_formula_bytes {
                return Err(EngineError::limit("Formula exceeds byte limit"));
            }
            formula::parse(&value.formula, limits.max_formula_nodes)?;
        }
        _ => {}
    }
    Ok(())
}

fn check_address(address: Address, limits: &Limits) -> Result<(), EngineError> {
    if address.column >= limits.max_columns || address.row >= limits.max_rows {
        return Err(EngineError::reference(
            "Address exceeds configured grid limits",
        ));
    }
    Ok(())
}

fn parse_address(value: &str, limits: &Limits) -> Result<Address, EngineError> {
    if value.len() > 64 {
        return Err(EngineError::reference("Address is too long"));
    }
    let address = Address::parse(value).map_err(EngineError::reference)?;
    check_address(address, limits)?;
    Ok(address)
}

fn parse_range(value: &str, limits: &Limits) -> Result<GridRange, EngineError> {
    if value.len() > 128 {
        return Err(EngineError::reference("Range is too long"));
    }
    let range = GridRange::parse(value).map_err(EngineError::reference)?;
    check_address(range.start, limits)?;
    check_address(range.end, limits)?;
    Ok(range)
}

fn bounds(sheet: &Sheet) -> Result<Option<GridRange>, EngineError> {
    let mut result: Option<GridRange> = None;
    for address in sheet.cells.keys() {
        let address = Address::parse(address).map_err(EngineError::reference)?;
        match &mut result {
            Some(range) => range.include(address),
            None => {
                result = Some(GridRange {
                    start: address,
                    end: address,
                })
            }
        }
    }
    Ok(result)
}

fn spend(budget: &mut usize, amount: usize) -> Result<(), EngineError> {
    *budget = budget
        .checked_sub(amount)
        .ok_or_else(|| EngineError::limit("Batch cell work budget exceeded"))?;
    Ok(())
}

struct MutationBudget {
    work: usize,
    cells: usize,
    text_bytes: usize,
}

fn cell_text_bytes(cell: &Cell) -> usize {
    match cell {
        Cell::Text(value) => value.len(),
        Cell::Formula(value) => value.formula.len(),
        _ => 0,
    }
}

fn value_text_bytes(value: &Value) -> usize {
    match value {
        Value::Text(value) => value.len(),
        Value::Formula(value) => value.formula.len(),
        Value::Error(value) => value.error.message.len(),
        _ => 0,
    }
}

fn check_response_size(value: &impl Serialize, limits: &Limits) -> Result<(), EngineError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| EngineError::new(ErrorCode::Serialization, error.to_string()))?;
    if bytes.len() > limits.max_response_bytes {
        return Err(EngineError::limit(
            "Result exceeds serialized byte limit; select fewer rows or columns",
        ));
    }
    Ok(())
}

fn select_columns(
    selected: Option<&[String]>,
    range: GridRange,
    limits: &Limits,
) -> Result<Vec<u32>, EngineError> {
    let columns = if let Some(selected) = selected {
        if selected.is_empty() || selected.len() > limits.max_read_cells {
            return Err(EngineError::limit(
                "Selected column count is outside limits",
            ));
        }
        selected
            .iter()
            .map(|name| parse_column(name).map_err(EngineError::input))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        if range.width() > limits.max_read_cells as u64 {
            return Err(EngineError::limit(
                "Range is wider than read cell budget; select columns",
            ));
        }
        (range.start.column..=range.end.column).collect()
    };
    let mut seen = BTreeSet::new();
    if columns.iter().any(|column| {
        *column < range.start.column || *column > range.end.column || !seen.insert(*column)
    }) {
        return Err(EngineError::input(
            "Selected columns must be unique and within the range",
        ));
    }
    Ok(columns)
}

fn validate_filter(filter: &Filter, limits: &Limits) -> Result<(), EngineError> {
    match &filter.value {
        Value::Number(value) if !value.is_finite() => {
            return Err(EngineError::input("Filter number must be finite"));
        }
        Value::Text(value) if value.len() > limits.max_cell_text_bytes => {
            return Err(EngineError::limit("Filter text exceeds byte limit"));
        }
        Value::Error(_) | Value::Formula(_) => {
            return Err(EngineError::input("Filter requires a primitive value"));
        }
        _ => {}
    }
    if filter.op == FilterOp::Contains && !matches!(filter.value, Value::Text(_)) {
        return Err(EngineError::input("Contains filter requires text"));
    }
    Ok(())
}

fn matches_filter(value: &Value, filter: &Filter) -> Result<bool, EngineError> {
    if filter.op == FilterOp::Contains {
        return Ok(
            matches!((value, &filter.value), (Value::Text(value), Value::Text(needle)) if value.contains(needle)),
        );
    }
    let comparison = match (value, &filter.value) {
        (Value::Blank, Value::Blank) => Some(Ordering::Equal),
        (Value::Number(a), Value::Number(b)) => a.partial_cmp(b),
        (Value::Text(a), Value::Text(b)) => Some(a.cmp(b)),
        (Value::Bool(a), Value::Bool(b)) => Some(a.cmp(b)),
        _ => None,
    };
    Ok(match filter.op {
        FilterOp::Eq => comparison == Some(Ordering::Equal),
        FilterOp::Ne => comparison != Some(Ordering::Equal),
        FilterOp::Lt => comparison == Some(Ordering::Less),
        FilterOp::Le => matches!(comparison, Some(Ordering::Less | Ordering::Equal)),
        FilterOp::Gt => comparison == Some(Ordering::Greater),
        FilterOp::Ge => matches!(comparison, Some(Ordering::Greater | Ordering::Equal)),
        FilterOp::Contains => false,
    })
}

fn compare_cells(left: &Cell, right: &Cell) -> Ordering {
    let rank = |cell: &Cell| match cell {
        Cell::Blank => 0,
        Cell::Bool(_) => 1,
        Cell::Number(_) => 2,
        Cell::Text(_) => 3,
        Cell::Formula(_) => 4,
    };
    match (left, right) {
        (Cell::Blank, Cell::Blank) => Ordering::Equal,
        (Cell::Bool(a), Cell::Bool(b)) => a.cmp(b),
        (Cell::Number(a), Cell::Number(b)) => a.total_cmp(b),
        (Cell::Text(a), Cell::Text(b)) => a.cmp(b),
        _ => rank(left).cmp(&rank(right)),
    }
}

fn changes(before: &Workbook, after: &Workbook) -> Result<(usize, Vec<ChangedRange>), EngineError> {
    let mut count = 0;
    let mut ranges = Vec::new();
    for (name, sheet) in &after.sheets {
        let previous = before.sheets.get(name);
        let keys = sheet
            .cells
            .keys()
            .chain(previous.into_iter().flat_map(|sheet| sheet.cells.keys()))
            .collect::<BTreeSet<_>>();
        let mut changed: Option<GridRange> = None;
        for key in keys {
            if sheet.cells.get(key) != previous.and_then(|sheet| sheet.cells.get(key)) {
                count += 1;
                let address = Address::parse(key).map_err(EngineError::reference)?;
                match &mut changed {
                    Some(range) => range.include(address),
                    None => {
                        changed = Some(GridRange {
                            start: address,
                            end: address,
                        })
                    }
                }
            }
        }
        if changed.is_some() || previous.is_none() {
            ranges.push(ChangedRange {
                sheet: name.clone(),
                range: changed.map(|range| range.to_string()),
            });
        }
    }
    Ok((count, ranges))
}

struct Evaluator<'a> {
    workbook: &'a Workbook,
    limits: &'a Limits,
    remaining: usize,
    remaining_value_bytes: usize,
    current_sheet: String,
    memo: HashMap<(String, Address), Result<Value, EngineError>>,
    active: HashSet<(String, Address)>,
    expressions: HashMap<String, Expr>,
}

impl<'a> Evaluator<'a> {
    fn new(workbook: &'a Workbook, limits: &'a Limits) -> Self {
        Self {
            workbook,
            limits,
            remaining: limits.max_evaluation_steps,
            remaining_value_bytes: limits.max_total_text_bytes,
            current_sheet: default_sheet(),
            memo: HashMap::new(),
            active: HashSet::new(),
            expressions: HashMap::new(),
        }
    }

    fn charge(&mut self) -> Result<(), EngineError> {
        self.remaining = self
            .remaining
            .checked_sub(1)
            .ok_or_else(|| EngineError::limit("Formula/read work budget exceeded"))?;
        Ok(())
    }

    fn cell_at(&mut self, sheet: &str, address: Address) -> Result<Value, EngineError> {
        self.charge()?;
        check_address(address, self.limits)?;
        let sheet = self.workbook.sheet_key(sheet)?.to_owned();
        let key = (sheet.clone(), address);
        if let Some(value) = self.memo.get(&key) {
            let bytes = value.as_ref().map(value_text_bytes).unwrap_or(0);
            self.remaining_value_bytes = self
                .remaining_value_bytes
                .checked_sub(bytes)
                .ok_or_else(|| EngineError::limit("Calculated text byte budget exceeded"))?;
            return value.clone();
        }
        if self.active.contains(&key) {
            return Err(EngineError::new(
                ErrorCode::Cycle,
                "Circular formula dependency",
            ));
        }
        if self.active.len() >= self.limits.max_dependency_depth {
            return Err(EngineError::limit("Formula dependency depth exceeded"));
        }
        let cell = self
            .workbook
            .sheets
            .get(&sheet)
            .and_then(|sheet| sheet.cells.get(&address.to_string()));
        let result = if let Some(Cell::Formula(formula)) = cell {
            let source = formula.formula.clone();
            let expression = match self.expressions.get(&source) {
                Some(expr) => expr.clone(),
                None => {
                    let expr = formula::parse(&source, self.limits.max_formula_nodes)?;
                    self.expressions.insert(source, expr.clone());
                    expr
                }
            };
            self.active.insert(key.clone());
            let previous_sheet = std::mem::replace(&mut self.current_sheet, sheet);
            let value = formula::evaluate(&expression, self);
            self.current_sheet = previous_sheet;
            self.active.remove(&key);
            value
        } else {
            Ok(cell.map(Value::from_cell).unwrap_or(Value::Blank))
        };
        if result.as_ref().is_ok_and(|value| matches!(value, Value::Text(text) if text.len() > self.limits.max_cell_text_bytes)) {
            return Err(EngineError::limit("Calculated cell text exceeds byte limit"));
        }
        let bytes = result.as_ref().map(value_text_bytes).unwrap_or(0);
        self.remaining_value_bytes = self
            .remaining_value_bytes
            .checked_sub(bytes)
            .ok_or_else(|| EngineError::limit("Calculated text byte budget exceeded"))?;
        self.memo.insert(key, result.clone());
        result
    }

    fn range_at(&mut self, sheet: &str, range: GridRange) -> Result<Vec<Value>, EngineError> {
        check_address(range.start, self.limits)?;
        check_address(range.end, self.limits)?;
        let area = usize::try_from(
            range
                .area()
                .ok_or_else(|| EngineError::limit("Formula range area overflow"))?,
        )
        .map_err(|_| EngineError::limit("Formula range too large"))?;
        if area > self.remaining {
            return Err(EngineError::limit(
                "Formula range exceeds remaining work budget",
            ));
        }
        let mut values = Vec::with_capacity(area);
        for row in range.start.row..=range.end.row {
            for column in range.start.column..=range.end.column {
                values.push(self.cell_at(sheet, Address { column, row })?);
            }
        }
        Ok(values)
    }
}

impl Context for Evaluator<'_> {
    fn step(&mut self) -> Result<(), EngineError> {
        self.charge()
    }
    fn cell(&mut self, reference: &Reference) -> Result<Value, EngineError> {
        let sheet = reference
            .sheet
            .as_deref()
            .unwrap_or(&self.current_sheet)
            .to_owned();
        self.cell_at(&sheet, reference.address)
    }
    fn range(&mut self, start: &Reference, end: &Reference) -> Result<Vec<Value>, EngineError> {
        let sheet = start
            .sheet
            .as_deref()
            .unwrap_or(&self.current_sheet)
            .to_owned();
        self.range_at(
            &sheet,
            GridRange {
                start: start.address,
                end: end.address,
            },
        )
    }
    fn named_range(&mut self, name: &str) -> Result<Vec<Value>, EngineError> {
        let range = self
            .workbook
            .names
            .get(&name.to_uppercase())
            .ok_or_else(|| EngineError::reference("Unknown named range"))?;
        self.range_at(&range.sheet, parse_range(&range.range, self.limits)?)
    }
}
