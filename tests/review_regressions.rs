use botus_1_2_3::{Cell, Limits, Operation, ReadRequest, Workbook};

#[test]
fn calculated_and_literal_text_share_the_cell_limit() -> Result<(), Box<dyn std::error::Error>> {
    let limits = Limits {
        max_cell_text_bytes: 5,
        ..Limits::default()
    };
    let mut workbook = Workbook::new();
    let original = workbook.clone();
    for cell in [
        Cell::Text("123456".into()),
        Cell::formula("=\"123456\""),
        Cell::formula("=IF(TRUE,\"123456\",\"ok\")"),
    ] {
        assert!(
            workbook
                .apply(
                    &[Operation::Put {
                        sheet: "Sheet1".into(),
                        at: "A1".into(),
                        values: vec![vec![cell]]
                    }],
                    &limits
                )
                .is_err()
        );
        assert_eq!(original, workbook);
    }
    let stored = br#"{"schema_version":1,"sheets":{"Sheet1":{"cells":{"A1":{"formula":"=\"123456\""}}}},"names":{}}"#;
    let loaded = Workbook::from_json(stored, &limits)?;
    assert!(loaded.read(&ReadRequest::default(), &limits).is_err());
    workbook.apply(
        &[Operation::Formula {
            sheet: "Sheet1".into(),
            at: "A1".into(),
            formula: "=IF(FALSE,\"123456\",\"ok\")".into(),
        }],
        &limits,
    )?;
    Ok(())
}

#[test]
fn duplicate_persisted_keys_never_overwrite_data() -> Result<(), Box<dyn std::error::Error>> {
    let limits = Limits::default();
    for input in [
        r#"{"schema_version":1,"sheets":{"Sheet1":{"cells":{"A1":1,"A1":2}}},"names":{}}"#,
        r#"{"schema_version":1,"sheets":{"Sheet1":{"cells":{"A1":1}},"Sheet1":{"cells":{"A1":2}}},"names":{}}"#,
        r#"{"schema_version":1,"sheets":{"Sheet1":{"cells":{}}},"names":{"TOTAL":{"sheet":"Sheet1","range":"A1"},"TOTAL":{"sheet":"Sheet1","range":"A2"}}}"#,
    ] {
        assert!(Workbook::from_json(input.as_bytes(), &limits).is_err());
    }
    let workbook = Workbook::new();
    assert_eq!(
        workbook,
        Workbook::from_json(&workbook.to_json(&limits)?, &limits)?
    );
    Ok(())
}
