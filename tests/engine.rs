use botus_1_2_3::{
    Cell, ErrorCode, Filter, FilterOp, Limits, Operation, ReadRequest, ReadView, SortKey, Value,
    Workbook,
};

fn put(at: &str, values: Vec<Vec<Cell>>) -> Operation {
    Operation::Put {
        sheet: "Sheet1".into(),
        at: at.into(),
        values,
    }
}
fn formula(at: &str, source: &str) -> Operation {
    Operation::Formula {
        sheet: "Sheet1".into(),
        at: at.into(),
        formula: source.into(),
    }
}
fn read(range: &str) -> ReadRequest {
    ReadRequest {
        range: Some(range.into()),
        ..ReadRequest::default()
    }
}

#[test]
fn names_qualified_sheets_and_dependency_chains() -> Result<(), Box<dyn std::error::Error>> {
    let limits = Limits::default();
    let mut workbook = Workbook::new();
    workbook.apply(
        &[
            Operation::CreateSheet {
                name: "Försäljning".into(),
            },
            Operation::Put {
                sheet: "försäljning".into(),
                at: "a1".into(),
                values: vec![vec![Cell::Number(4.0)], vec![Cell::Number(6.0)]],
            },
            Operation::Name {
                name: "revenue".into(),
                sheet: "Försäljning".into(),
                range: "a1..a2".into(),
            },
            formula("A1", "=@SUM(revenue)"),
            formula("B1", "=A1*2"),
            formula("C1", "=AVERAGE('Försäljning'!A1:A2)+B1"),
        ],
        &limits,
    )?;
    assert_eq!(
        workbook.read(&read("A1:C1"), &limits)?.rows,
        vec![vec![
            Value::Number(10.0),
            Value::Number(20.0),
            Value::Number(25.0)
        ]]
    );
    assert!(workbook.names.contains_key("REVENUE"));
    let original = workbook.clone();
    assert_eq!(
        workbook
            .apply(&[formula("A1", "=B1")], &limits)
            .err()
            .ok_or("expected cycle")?
            .code,
        ErrorCode::Cycle
    );
    assert_eq!(original, workbook);
    Ok(())
}

#[test]
fn anchors_fill_and_cross_sheet_copy() -> Result<(), Box<dyn std::error::Error>> {
    let limits = Limits::default();
    let mut workbook = Workbook::new();
    workbook.apply(
        &[
            put(
                "A1",
                (1..=5).map(|n| vec![Cell::Number(f64::from(n))]).collect(),
            ),
            put("C1", vec![vec![Cell::Number(10.0)]]),
            formula("B1", "=A1+$C$1"),
            Operation::Fill {
                sheet: "Sheet1".into(),
                range: "B1".into(),
                to: "B2:B5".into(),
            },
            Operation::CreateSheet {
                name: "Floppy futures".into(),
            },
            Operation::Copy {
                sheet: "Sheet1".into(),
                range: "A1:C5".into(),
                to: "A1".into(),
                target_sheet: Some("Floppy futures".into()),
            },
        ],
        &limits,
    )?;
    assert_eq!(
        workbook.read(&read("B1:B5"), &limits)?.rows,
        (11..=15)
            .map(|n| vec![Value::Number(f64::from(n))])
            .collect::<Vec<_>>()
    );
    let copied = workbook.read(
        &ReadRequest {
            sheet: "Floppy futures".into(),
            range: Some("B1:B5".into()),
            ..ReadRequest::default()
        },
        &limits,
    )?;
    assert_eq!(copied.rows[4], vec![Value::Number(15.0)]);
    let formula_view = workbook.read(
        &ReadRequest {
            view: ReadView::Formulas,
            ..read("B2")
        },
        &limits,
    )?;
    assert_eq!(
        formula_view.rows,
        vec![vec![Value::Formula(botus_1_2_3::Formula {
            formula: "=(A2+$C$1)".into()
        })]]
    );
    Ok(())
}

#[test]
fn overlapping_copy_and_fill_use_final_text_accounting() -> Result<(), Box<dyn std::error::Error>> {
    let limits = Limits {
        max_total_text_bytes: 2,
        ..Limits::default()
    };
    let mut workbook = Workbook::new();
    workbook.apply(
        &[put(
            "A1",
            vec![vec![
                Cell::Text("a".into()),
                Cell::Blank,
                Cell::Text("b".into()),
            ]],
        )],
        &limits,
    )?;
    workbook.apply(
        &[Operation::Copy {
            sheet: "Sheet1".into(),
            range: "A1:B1".into(),
            to: "B1".into(),
            target_sheet: None,
        }],
        &limits,
    )?;
    assert_eq!(
        workbook.read(&read("A1:C1"), &limits)?.rows,
        vec![vec![
            Value::Text("a".into()),
            Value::Text("a".into()),
            Value::Blank
        ]]
    );
    let mut workbook = Workbook::new();
    workbook.apply(
        &[put(
            "A1",
            vec![vec![
                Cell::Text("a".into()),
                Cell::Blank,
                Cell::Text("b".into()),
            ]],
        )],
        &limits,
    )?;
    workbook.apply(
        &[Operation::Fill {
            sheet: "Sheet1".into(),
            range: "A1:B1".into(),
            to: "A1:D1".into(),
        }],
        &limits,
    )?;
    assert_eq!(
        workbook.read(&read("A1:D1"), &limits)?.rows,
        vec![vec![
            Value::Text("a".into()),
            Value::Blank,
            Value::Text("a".into()),
            Value::Blank
        ]]
    );
    let before = workbook.clone();
    assert!(
        workbook
            .apply(
                &[Operation::Fill {
                    sheet: "Sheet1".into(),
                    range: "A1".into(),
                    to: "A1:A10000".into()
                }],
                &limits
            )
            .is_err()
    );
    assert_eq!(before, workbook);
    Ok(())
}

#[test]
fn sort_is_stable_and_formulas_cannot_move_implicitly() -> Result<(), Box<dyn std::error::Error>> {
    let limits = Limits::default();
    let mut workbook = Workbook::new();
    workbook.apply(
        &[put(
            "A1",
            vec![
                vec![
                    Cell::Text("Department of recalculation".into()),
                    Cell::Text("Rank".into()),
                ],
                vec![Cell::Text("Second floppy".into()), Cell::Number(2.0)],
                vec![Cell::Text("First floppy".into()), Cell::Number(1.0)],
                vec![Cell::Text("Spare floppy".into()), Cell::Number(2.0)],
            ],
        )],
        &limits,
    )?;
    let sort = Operation::Sort {
        sheet: "Sheet1".into(),
        range: "A1:B4".into(),
        keys: vec![SortKey {
            column: "B".into(),
            descending: false,
        }],
        header: true,
    };
    workbook.apply(std::slice::from_ref(&sort), &limits)?;
    let rows = workbook.read(&read("A1:B4"), &limits)?.rows;
    assert_eq!(rows[1][0], Value::Text("First floppy".into()));
    assert_eq!(rows[2][0], Value::Text("Second floppy".into()));
    assert_eq!(rows[3][0], Value::Text("Spare floppy".into()));
    workbook.apply(&[formula("B2", "=1")], &limits)?;
    let before = workbook.clone();
    assert!(workbook.apply(&[sort], &limits).is_err());
    assert_eq!(before, workbook);
    Ok(())
}

#[test]
fn sparse_implicit_read_paginates_without_dense_allocation()
-> Result<(), Box<dyn std::error::Error>> {
    let mut workbook = Workbook::new();
    let limits = Limits {
        max_read_cells: 8,
        max_scan_rows: 3,
        ..Limits::default()
    };
    workbook.apply(
        &[
            put("A1", vec![vec![Cell::Number(1.0)]]),
            put(
                "AMJ100000",
                vec![vec![Cell::Text("The last floppy".into())]],
            ),
        ],
        &limits,
    )?;
    assert!(workbook.read(&ReadRequest::default(), &limits).is_err());
    let page = workbook.read(
        &ReadRequest {
            columns: Some(vec!["A".into(), "AMJ".into()]),
            ..ReadRequest::default()
        },
        &limits,
    )?;
    assert_eq!(page.rows.len(), 3);
    assert_eq!(page.next_offset, Some(3));
    let last = workbook.read(
        &ReadRequest {
            columns: Some(vec!["AMJ".into()]),
            offset: 99_999,
            ..ReadRequest::default()
        },
        &limits,
    )?;
    assert_eq!(last.row_numbers, vec![100_000]);
    assert_eq!(last.rows, vec![vec![Value::Text("The last floppy".into())]]);
    let filtered = workbook.read(
        &ReadRequest {
            columns: Some(vec!["A".into()]),
            filter: Some(Filter {
                column: "AMJ".into(),
                op: FilterOp::Contains,
                value: Value::Text("floppy".into()),
            }),
            ..ReadRequest::default()
        },
        &limits,
    )?;
    assert!(filtered.rows.is_empty());
    assert_eq!(filtered.next_offset, Some(3));
    Ok(())
}

#[test]
fn serialized_bytes_and_calculated_text_are_bounded() -> Result<(), Box<dyn std::error::Error>> {
    let mut workbook = Workbook::new();
    let limits = Limits {
        max_total_text_bytes: 1024,
        max_cell_text_bytes: 1024,
        max_response_bytes: 512,
        ..Limits::default()
    };
    workbook.apply(
        &[put("A1", vec![vec![Cell::Text("\"".repeat(250))]])],
        &limits,
    )?;
    assert!(workbook.read(&read("A1"), &limits).is_err());
    let larger_output = Limits {
        max_response_bytes: 4096,
        ..limits.clone()
    };
    assert!(workbook.read(&read("A1"), &larger_output).is_ok());
    let before = workbook.clone();
    assert!(
        workbook
            .apply(
                &[
                    formula("B1", "=A1"),
                    formula("C1", "=A1"),
                    formula("D1", "=A1"),
                    formula("E1", "=A1"),
                    formula("F1", "=A1"),
                ],
                &limits
            )
            .is_err()
    );
    assert_eq!(before, workbook);
    assert!(
        workbook
            .to_json(&Limits {
                max_document_bytes: 100,
                ..limits
            })
            .is_err()
    );
    Ok(())
}

#[test]
fn malformed_stored_data_and_unknown_fields_are_rejected() -> Result<(), Box<dyn std::error::Error>>
{
    let limits = Limits::default();
    let workbook = Workbook::new();
    let mut document: serde_json::Value = serde_json::from_slice(&workbook.to_json(&limits)?)?;
    document["schema_version"] = serde_json::json!(2);
    assert_eq!(
        Workbook::from_json(&serde_json::to_vec(&document)?, &limits)
            .err()
            .ok_or("expected version error")?
            .code,
        ErrorCode::Version
    );
    document["schema_version"] = serde_json::json!(1);
    document["sheets"]["Sheet1"]["cells"] = serde_json::json!({"a1":4});
    assert!(Workbook::from_json(&serde_json::to_vec(&document)?, &limits).is_err());
    document["sheets"]["Sheet1"]["cells"] = serde_json::json!({"A0":4});
    assert!(Workbook::from_json(&serde_json::to_vec(&document)?, &limits).is_err());
    assert!(
        serde_json::from_value::<Operation>(
            serde_json::json!({"op":"put","at":"A1","values":[[1]],"actor":"impostor"})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<Cell>(serde_json::json!({"formula":"=1","other":true})).is_err()
    );
    Ok(())
}

#[test]
fn imported_formula_errors_are_distinct_and_unsupported_syntax_rejects()
-> Result<(), Box<dyn std::error::Error>> {
    let limits = Limits::default();
    let workbook = Workbook::from_json(br#"{"schema_version":1,"sheets":{"Sheet1":{"cells":{"A1":{"formula":"=1/0"},"B1":"Division by zero"}}},"names":{}}"#, &limits)?;
    let result = workbook.read(&read("A1:B1"), &limits)?;
    assert!(
        matches!(&result.rows[0][0], Value::Error(value) if value.error.code == ErrorCode::DivideByZero)
    );
    assert_eq!(result.rows[0][1], Value::Text("Division by zero".into()));
    assert!(Workbook::from_json(br#"{"schema_version":1,"sheets":{"Sheet1":{"cells":{"A1":{"formula":"=SHELL(1)"}}}},"names":{}}"#, &limits).is_err());
    Ok(())
}
