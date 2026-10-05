use botus_1_2_3::{ErrorCode, Limits, Operation, ReadRequest, Value, Workbook};
use serde_json::json;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn operations(value: serde_json::Value) -> Result<Vec<Operation>, serde_json::Error> {
    serde_json::from_value(value)
}

#[test]
fn round_trip_keeps_literal_formulas_and_distinct_empty_values() -> TestResult {
    let limits = Limits::default();
    let mut book = Workbook::new();
    book.apply(
        &operations(json!([{
            "op": "put", "at": "A1",
            "values": [[null, "", 0, false, "=SUM(A1:A3)", "lotus \u{1f338}"]]
        }]))?,
        &limits,
    )?;
    let loaded = Workbook::from_json(&book.to_json(&limits)?, &limits)?;
    let result = loaded.read(
        &ReadRequest {
            range: Some("A1:F1".into()),
            ..ReadRequest::default()
        },
        &limits,
    )?;
    assert_eq!(
        result.rows,
        vec![vec![
            Value::Blank,
            Value::Text(String::new()),
            Value::Number(0.0),
            Value::Bool(false),
            Value::Text("=SUM(A1:A3)".into()),
            Value::Text("lotus \u{1f338}".into()),
        ]]
    );
    Ok(())
}

#[test]
fn rejected_last_operation_does_not_commit_earlier_cells() -> TestResult {
    let limits = Limits::default();
    let mut book = Workbook::new();
    let before = book.to_json(&limits)?;
    let result = book.apply(
        &operations(json!([
            {"op": "put", "at": "A1", "values": [[99]]},
            {"op": "formula", "at": "B1", "formula": "=1/0"}
        ]))?,
        &limits,
    );
    assert!(result.is_err());
    assert_eq!(book.to_json(&limits)?, before);
    Ok(())
}

#[test]
fn read_work_budget_covers_the_whole_page() -> TestResult {
    let mut book = Workbook::new();
    book.apply(
        &operations(json!([{"op": "put", "at": "A1", "values": [[1], [2], [3]]}]))?,
        &Limits::default(),
    )?;
    let limits = Limits {
        max_evaluation_steps: 2,
        ..Limits::default()
    };
    let result = book.read(
        &ReadRequest {
            range: Some("A1:A3".into()),
            ..ReadRequest::default()
        },
        &limits,
    );
    assert!(matches!(result, Err(error) if error.code == ErrorCode::Limit));
    Ok(())
}

#[test]
fn lazy_if_does_not_visit_unselected_self_reference() -> TestResult {
    let limits = Limits::default();
    let mut book = Workbook::new();
    book.apply(
        &operations(json!([{"op": "formula", "at": "A1", "formula": "=IF(FALSE,A1,7)"}]))?,
        &limits,
    )?;
    assert_eq!(
        book.read(&ReadRequest::default(), &limits)?.rows,
        vec![vec![Value::Number(7.0)]]
    );
    Ok(())
}

#[test]
fn flat_operator_chain_is_bounded_like_nested_expressions() -> TestResult {
    let limits = Limits::default();
    let mut book = Workbook::new();
    let source = format!("={}1", "1+".repeat(100));
    let before = book.to_json(&limits)?;
    let result = book.apply(
        &operations(json!([{"op": "formula", "at": "A1", "formula": source}]))?,
        &limits,
    );
    assert!(matches!(result, Err(error) if error.code == ErrorCode::Limit));
    assert_eq!(book.to_json(&limits)?, before);
    Ok(())
}
