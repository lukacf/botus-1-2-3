//! Behavioral cases from SPEC.md and the bundled read/operation/formula manual.
//! Expected values are fixed examples, independent of the engine's evaluator.

use botus_1_2_3::{
    Cell, EngineError, ErrorCode, Filter, FilterOp, Formula, Limits, Operation, ReadRequest,
    ReadView, Value, Workbook,
};
use serde_json::json;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn operations(value: serde_json::Value) -> Result<Vec<Operation>, serde_json::Error> {
    serde_json::from_value(value)
}

fn read(range: &str) -> ReadRequest {
    ReadRequest {
        range: Some(range.into()),
        ..ReadRequest::default()
    }
}

fn refused<T>(result: Result<T, EngineError>, expected: ErrorCode) -> TestResult {
    let error = result.err().ok_or("expected the operation to be refused")?;
    assert_eq!(error.code, expected, "{error}");
    Ok(())
}

#[test]
fn clear_removes_only_its_rectangle_and_recalculates_remaining_formulas() -> TestResult {
    let limits = Limits::default();
    let mut book = Workbook::new();
    book.apply(
        &operations(json!([
            {"op":"put","at":"A1","values":[
                [2,{"formula":"=A1*3"},"keep"],["erase",true,11]
            ]},
            {"op":"formula","at":"D1","formula":"=SUM(A1:B2)"},
            {"op":"put","at":"Z99","values":[[5]]},
            {"op":"name","name":"cleared_area","range":"A1:B2"}
        ]))?,
        &limits,
    )?;
    let names = book.names.clone();
    let clear = operations(json!([{"op":"clear","range":"a1:b2"}]))?;
    let receipt = book.apply(&clear, &limits)?;
    assert_eq!(receipt.changed_cells, 4);
    assert_eq!(receipt.total_cells, 4);
    assert_eq!(receipt.changed_ranges.len(), 1);
    assert_eq!(receipt.changed_ranges[0].range.as_deref(), Some("A1:B2"));
    assert_eq!(book.names, names);
    assert_eq!(
        book.read(&read("A1:D2"), &limits)?.rows,
        vec![
            vec![
                Value::Blank,
                Value::Blank,
                Value::Text("keep".into()),
                Value::Number(0.0)
            ],
            vec![
                Value::Blank,
                Value::Blank,
                Value::Number(11.0),
                Value::Blank
            ],
        ],
    );
    assert_eq!(
        book.read(&read("Z99"), &limits)?.rows,
        vec![vec![Value::Number(5.0)]]
    );
    let before_noop = book.to_json(&limits)?;
    assert_eq!(book.apply(&clear, &limits)?.changed_cells, 0);
    assert_eq!(book.to_json(&limits)?, before_noop);
    Ok(())
}

#[test]
fn clear_that_breaks_a_remaining_formula_rolls_back_the_whole_batch() -> TestResult {
    let limits = Limits::default();
    let mut book = Workbook::new();
    book.apply(
        &operations(json!([
            {"op":"put","at":"A1","values":[[2,{"formula":"=10/A1"}]]}
        ]))?,
        &limits,
    )?;
    let before = book.to_json(&limits)?;
    refused(
        book.apply(
            &operations(json!([
                {"op":"put","at":"C1","values":[[99]]},
                {"op":"clear","range":"A1"}
            ]))?,
            &limits,
        ),
        ErrorCode::DivideByZero,
    )?;
    assert_eq!(book.to_json(&limits)?, before);
    assert_eq!(
        book.read(&read("A1:C1"), &limits)?.rows,
        vec![vec![Value::Number(2.0), Value::Number(5.0), Value::Blank]]
    );
    Ok(())
}

#[test]
fn min_max_count_ignore_nonnumbers_and_define_empty_results() -> TestResult {
    let limits = Limits::default();
    let mut book = Workbook::new();
    book.apply(
        &operations(json!([
            {"op":"put","at":"A1","values":[[-4],[9],["100"],[true],[null],[3]]},
            {"op":"put","at":"C1","values":[
                [{"formula":"=MIN(A1:A6,2,FALSE,\"-99\")"},
                 {"formula":"=MAX(A1:A6,2,FALSE,\"100\")"},
                 {"formula":"=COUNT(A1:A6,2,FALSE,\"100\")"}],
                [{"formula":"=MIN()"},{"formula":"=MAX()"},{"formula":"=COUNT()"}],
                [{"formula":"=MIN(TRUE,\"7\",Z1:Z2)"},
                 {"formula":"=MAX(TRUE,\"7\",Z1:Z2)"},
                 {"formula":"=COUNT(TRUE,\"7\",Z1:Z2)"}]
            ]}
        ]))?,
        &limits,
    )?;
    assert_eq!(
        book.read(&read("C1:E3"), &limits)?.rows,
        vec![
            vec![Value::Number(-4.0), Value::Number(9.0), Value::Number(4.0)],
            vec![Value::Number(0.0); 3],
            vec![Value::Number(0.0); 3],
        ]
    );
    Ok(())
}

#[test]
fn subtraction_division_and_parentheses_follow_numeric_precedence() -> TestResult {
    let limits = Limits::default();
    let mut book = Workbook::new();
    for (source, expected) in [
        ("=20-6-4", 10.0),
        ("=20-(6-4)", 18.0),
        ("=24/3/2", 4.0),
        ("=18/(9/3)", 6.0),
        ("=(20-6)/(3+4)", 2.0),
        ("=3-(2+4)*2", -9.0),
    ] {
        book.apply(
            &operations(json!([
                {"op":"formula","at":"A1","formula":source}
            ]))?,
            &limits,
        )?;
        assert_eq!(
            book.read(&read("A1"), &limits)?.rows,
            vec![vec![Value::Number(expected)]],
            "{source}"
        );
    }
    Ok(())
}

#[test]
fn comparisons_return_booleans_and_do_not_parse_numeric_text() -> TestResult {
    let limits = Limits::default();
    let mut book = Workbook::new();
    for (source, expected) in [
        ("=3=3", true),
        ("=3=4", false),
        ("=3<>4", true),
        ("=3<>3", false),
        ("=3<4", true),
        ("=3<3", false),
        ("=3<=3", true),
        ("=4<=3", false),
        ("=4>3", true),
        ("=3>3", false),
        ("=3>=3", true),
        ("=3>=4", false),
        ("=TRUE>FALSE", true),
        ("=B1=0", true),
    ] {
        book.apply(
            &operations(json!([
                {"op":"formula","at":"A1","formula":source}
            ]))?,
            &limits,
        )?;
        assert_eq!(
            book.read(&read("A1"), &limits)?.rows,
            vec![vec![Value::Bool(expected)]],
            "{source}"
        );
    }
    let before = book.to_json(&limits)?;
    for source in ["=\"3\"=3", "=\"3\"<4"] {
        refused(
            book.apply(
                &operations(json!([
                    {"op":"formula","at":"A1","formula":source}
                ]))?,
                &limits,
            ),
            ErrorCode::FormulaType,
        )?;
        assert_eq!(book.to_json(&limits)?, before);
    }
    Ok(())
}

#[test]
fn computed_nonfinite_values_reject_writes_and_remain_distinct_read_errors() -> TestResult {
    let limits = Limits::default();
    let mut book = Workbook::new();
    book.apply(
        &operations(json!([
            {"op":"put","at":"A1","values":[["not an error object"]]}
        ]))?,
        &limits,
    )?;
    let before = book.to_json(&limits)?;
    for source in [
        "=1e308*2",
        "=1e308/1e-308",
        "=10^400",
        "=(-1)^0.5",
        // Mathematically the third term cancels overflow, but the documented
        // intermediate-sum rule refuses it before that term is visited.
        "=SUM(1e308,1e308,-1e308)",
        "=AVERAGE(1e308,1e308,-1e308)",
    ] {
        refused(
            book.apply(
                &operations(json!([
                    {"op":"formula","at":"B1","formula":source}
                ]))?,
                &limits,
            ),
            ErrorCode::NonFinite,
        )?;
        assert_eq!(book.to_json(&limits)?, before, "{source}");

        let mut stored: serde_json::Value = serde_json::from_slice(&before)?;
        stored["sheets"]["Sheet1"]["cells"]["B1"] = json!({"formula":source});
        let imported = Workbook::from_json(&serde_json::to_vec(&stored)?, &limits)?;
        let output = imported.read(&read("A1:B1"), &limits)?;
        assert_eq!(output.rows[0][0], Value::Text("not an error object".into()));
        assert!(
            matches!(&output.rows[0][1], Value::Error(value)
            if value.error.code == ErrorCode::NonFinite),
            "{source}"
        );
    }
    Ok(())
}

fn filter_book() -> Result<Workbook, Box<dyn std::error::Error>> {
    let mut book = Workbook::new();
    book.apply(
        &operations(json!([
            {"op":"put","at":"A1","values":[
                ["low",-2,100],["zero",0,200],["two",2,300],["text-two","2",400],
                ["false",false,500],["true",true,600],["blank",null,700],
                ["upper","Floppy",800],["lower","floppy disk",900],
                ["calculated",{"formula":"=1+1"},1000]
            ]}
        ]))?,
        &Limits::default(),
    )?;
    Ok(book)
}

#[test]
fn typed_filters_match_without_cross_type_coercion_and_keep_column_order() -> TestResult {
    let book = filter_book()?;
    let limits = Limits::default();
    for (op, value, rows) in [
        (FilterOp::Eq, Value::Number(2.0), vec![3, 10]),
        (
            FilterOp::Ne,
            Value::Number(2.0),
            vec![1, 2, 4, 5, 6, 7, 8, 9],
        ),
        (FilterOp::Lt, Value::Number(2.0), vec![1, 2]),
        (FilterOp::Le, Value::Number(2.0), vec![1, 2, 3, 10]),
        (FilterOp::Gt, Value::Number(0.0), vec![3, 10]),
        (FilterOp::Ge, Value::Number(0.0), vec![2, 3, 10]),
        (FilterOp::Eq, Value::Number(0.0), vec![2]),
        (FilterOp::Eq, Value::Text("2".into()), vec![4]),
        (FilterOp::Eq, Value::Bool(false), vec![5]),
        (FilterOp::Lt, Value::Bool(true), vec![5]),
        (FilterOp::Ge, Value::Bool(false), vec![5, 6]),
        (FilterOp::Eq, Value::Blank, vec![7]),
        (FilterOp::Ne, Value::Blank, vec![1, 2, 3, 4, 5, 6, 8, 9, 10]),
        (FilterOp::Lt, Value::Text("floppy".into()), vec![4, 8]),
        (FilterOp::Gt, Value::Text("Floppy".into()), vec![9]),
        (FilterOp::Contains, Value::Text("floppy".into()), vec![9]),
    ] {
        let result = book.read(
            &ReadRequest {
                columns: Some(vec!["c".into(), "a".into()]),
                filter: Some(Filter {
                    column: "b".into(),
                    op,
                    value: value.clone(),
                }),
                ..read("A1:C10")
            },
            &limits,
        )?;
        assert_eq!(result.row_numbers, rows, "{op:?} {value:?}");
        assert_eq!(result.columns, ["C", "A"]);
    }
    // Filtering calculates B10 even when the returned view preserves formulas.
    let filtered = book.read(
        &ReadRequest {
            columns: Some(vec!["b".into(), "a".into(), "c".into()]),
            view: ReadView::Formulas,
            filter: Some(Filter {
                column: "B".into(),
                op: FilterOp::Eq,
                value: Value::Number(2.0),
            }),
            ..read("A1:C10")
        },
        &limits,
    )?;
    assert_eq!(filtered.columns, ["B", "A", "C"]);
    assert_eq!(
        filtered.rows,
        vec![
            vec![
                Value::Number(2.0),
                Value::Text("two".into()),
                Value::Number(300.0)
            ],
            vec![
                Value::Formula(Formula {
                    formula: "=1+1".into()
                }),
                Value::Text("calculated".into()),
                Value::Number(1000.0)
            ],
        ]
    );
    Ok(())
}

#[test]
fn invalid_output_and_filter_columns_are_refused() -> TestResult {
    let book = filter_book()?;
    let limits = Limits::default();
    for columns in [vec!["A", "a"], vec!["D"], vec!["1"], vec!["A1"], vec![""]] {
        refused(
            book.read(
                &ReadRequest {
                    columns: Some(columns.into_iter().map(str::to_owned).collect()),
                    ..read("A1:C10")
                },
                &limits,
            ),
            ErrorCode::InvalidInput,
        )?;
    }
    refused(
        book.read(
            &ReadRequest {
                columns: Some(vec![]),
                ..read("A1:C10")
            },
            &limits,
        ),
        ErrorCode::Limit,
    )?;
    for column in ["D", "B1", ""] {
        refused(
            book.read(
                &ReadRequest {
                    filter: Some(Filter {
                        column: column.into(),
                        op: FilterOp::Eq,
                        value: Value::Blank,
                    }),
                    ..read("A1:C10")
                },
                &limits,
            ),
            ErrorCode::InvalidInput,
        )?;
    }
    for value in [Value::Number(2.0), Value::Blank, Value::Bool(true)] {
        refused(
            book.read(
                &ReadRequest {
                    filter: Some(Filter {
                        column: "B".into(),
                        op: FilterOp::Contains,
                        value,
                    }),
                    ..read("A1:C10")
                },
                &limits,
            ),
            ErrorCode::InvalidInput,
        )?;
    }
    Ok(())
}

#[test]
fn a_formula_error_in_an_unselected_filter_column_fails_the_read() -> TestResult {
    let limits = Limits::default();
    let stored = json!({"schema_version":1,"sheets":{"Sheet1":{"cells":{
        "A1":"record", "B1":{"formula":"=1/0"}
    }}},"names":{}});
    let book = Workbook::from_json(&serde_json::to_vec(&stored)?, &limits)?;
    for view in [ReadView::Values, ReadView::Formulas] {
        refused(
            book.read(
                &ReadRequest {
                    columns: Some(vec!["A".into()]),
                    view,
                    filter: Some(Filter {
                        column: "B".into(),
                        op: FilterOp::Ne,
                        value: Value::Number(0.0),
                    }),
                    ..read("A1:B1")
                },
                &limits,
            ),
            ErrorCode::DivideByZero,
        )?;
    }
    Ok(())
}

#[test]
fn dependency_depth_is_bounded_in_reads_and_atomic_mutations() -> TestResult {
    let at_bound = Limits {
        max_dependency_depth: 3,
        ..Limits::default()
    };
    let too_shallow = Limits {
        max_dependency_depth: 2,
        ..at_bound.clone()
    };
    let chain = operations(json!([
        {"op":"put","at":"A1","values":[[{"formula":"=B1+1"},{"formula":"=C1+1"},7]]}
    ]))?;
    let mut book = Workbook::new();
    let empty = book.to_json(&at_bound)?;
    refused(book.apply(&chain, &too_shallow), ErrorCode::Limit)?;
    assert_eq!(book.to_json(&at_bound)?, empty);
    book.apply(&chain, &at_bound)?;
    let before_read = book.to_json(&at_bound)?;
    assert_eq!(
        book.read(&read("A1"), &at_bound)?.rows,
        vec![vec![Value::Number(9.0)]]
    );
    refused(book.read(&read("A1"), &too_shallow), ErrorCode::Limit)?;
    assert_eq!(book.to_json(&at_bound)?, before_read);
    Ok(())
}

#[test]
fn shared_dependencies_reuse_work_without_persisting_a_stale_calculation() -> TestResult {
    // This envelope leaves ample room to sum 1..=128 once and follow four
    // references, but not to repeat that aggregate for every selected cell.
    // It intentionally does not assert the evaluator's exact step accounting.
    let limits = Limits {
        max_evaluation_steps: 600,
        ..Limits::default()
    };
    let mut book = Workbook::new();
    let mut batch = vec![Operation::Put {
        sheet: "Sheet1".into(),
        at: "C1".into(),
        values: (1..=128)
            .map(|n| vec![Cell::Number(f64::from(n))])
            .collect(),
    }];
    batch.extend(operations(json!([
        {"op":"formula","at":"A1","formula":"=SUM(C1:C128)"},
        {"op":"put","at":"B1","values":[
            [{"formula":"=A1+1"}],[{"formula":"=A1+2"}],
            [{"formula":"=A1+3"}],[{"formula":"=A1+4"}]
        ]}
    ]))?);
    // Mutation validation must share its memo too.
    book.apply(&batch, &limits)?;
    let before_read = book.to_json(&limits)?;
    assert_eq!(
        book.read(&read("B1:B4"), &limits)?.rows,
        vec![
            vec![Value::Number(8257.0)],
            vec![Value::Number(8258.0)],
            vec![Value::Number(8259.0)],
            vec![Value::Number(8260.0)],
        ]
    );
    assert_eq!(book.to_json(&limits)?, before_read);
    book.apply(
        &operations(json!([
            {"op":"put","at":"C1","values":[[101]]}
        ]))?,
        &limits,
    )?;
    assert_eq!(
        book.read(&read("B1:B4"), &limits)?.rows,
        vec![
            vec![Value::Number(8357.0)],
            vec![Value::Number(8358.0)],
            vec![Value::Number(8359.0)],
            vec![Value::Number(8360.0)],
        ]
    );
    Ok(())
}

#[test]
fn mutation_work_is_cumulative_even_when_writes_replace_the_same_cell() -> TestResult {
    let one = Limits {
        max_operation_cells: 1,
        ..Limits::default()
    };
    let two = Limits {
        max_operation_cells: 2,
        ..one.clone()
    };
    let batch = operations(json!([
        {"op":"put","at":"A1","values":[[2]]},
        {"op":"formula","at":"A1","formula":"=3"}
    ]))?;
    // Both operations are individually legal under the smaller work budget.
    for operation in &batch {
        Workbook::new().apply(std::slice::from_ref(operation), &one)?;
    }
    let mut book = Workbook::new();
    let before = book.to_json(&two)?;
    refused(book.apply(&batch, &one), ErrorCode::Limit)?;
    assert_eq!(book.to_json(&two)?, before);
    let result = book.apply(&batch, &two)?;
    assert_eq!(result.operations, 2);
    assert_eq!(result.changed_cells, 1);
    assert_eq!(
        book.read(&read("A1"), &two)?.rows,
        vec![vec![Value::Number(3.0)]]
    );

    // Blank matrix entries still represent work even when they store no cells.
    let blanks = operations(json!([{"op":"put","at":"B1","values":[[null,null,null]]}]))?;
    let before = book.to_json(&two)?;
    refused(book.apply(&blanks, &two), ErrorCode::Limit)?;
    assert_eq!(book.to_json(&two)?, before);
    let three = Limits {
        max_operation_cells: 3,
        ..two
    };
    assert_eq!(book.apply(&blanks, &three)?.changed_cells, 0);
    Ok(())
}

#[test]
fn clear_charges_a_sparse_sheet_scan_and_shares_the_batch_budget() -> TestResult {
    let limits = Limits::default();
    let mut book = Workbook::new();
    book.apply(
        &operations(json!([
            {"op":"put","at":"A1","values":[[1,null,3,null,5]]}
        ]))?,
        &limits,
    )?;
    let batch = operations(json!([
        {"op":"clear","range":"A1:A100000"},
        {"op":"put","at":"B1","values":[[2]]}
    ]))?;
    let three = Limits {
        max_operation_cells: 3,
        ..limits.clone()
    };
    let before = book.to_json(&limits)?;
    refused(book.apply(&batch, &three), ErrorCode::Limit)?;
    assert_eq!(book.to_json(&limits)?, before);
    let four = Limits {
        max_operation_cells: 4,
        ..limits
    };
    let receipt = book.apply(&batch, &four)?;
    assert_eq!(receipt.changed_cells, 2);
    assert_eq!(receipt.total_cells, 3);
    assert_eq!(
        book.read(&read("A1:E1"), &four)?.rows,
        vec![vec![
            Value::Blank,
            Value::Number(2.0),
            Value::Number(3.0),
            Value::Blank,
            Value::Number(5.0)
        ]]
    );
    Ok(())
}

#[test]
fn sheet_cell_and_batch_count_bounds_preserve_existing_content() -> TestResult {
    let limits = Limits {
        max_sheets: 2,
        max_cells: 3,
        max_batch_operations: 2,
        ..Limits::default()
    };
    let mut book = Workbook::new();
    book.apply(
        &operations(json!([
            {"op":"create_sheet","name":"Data"},
            {"op":"put","at":"A1","values":[[0,null,false]]}
        ]))?,
        &limits,
    )?;
    let receipt = book.apply(
        &operations(json!([
            {"op":"put","sheet":"Data","at":"A1","values":[[""]]}
        ]))?,
        &limits,
    )?;
    assert_eq!(receipt.sheets, 2);
    assert_eq!(receipt.total_cells, 3);
    let before = book.to_json(&limits)?;
    for batch in [
        json!([{"op":"create_sheet","name":"Third"}]),
        // A later clear cannot excuse exceeding the intermediate cell bound.
        json!([
            {"op":"put","sheet":"Data","at":"A2","values":[[4]]},
            {"op":"clear","range":"A1"}
        ]),
        // These valid zero-change operations exceed only the operation count.
        json!([
            {"op":"clear","range":"Z1"},
            {"op":"clear","range":"Z2"},
            {"op":"clear","range":"Z3"}
        ]),
    ] {
        refused(book.apply(&operations(batch)?, &limits), ErrorCode::Limit)?;
        assert_eq!(book.to_json(&limits)?, before);
    }
    // Releasing a cell first permits another sheet to use that capacity.
    assert_eq!(
        book.apply(
            &operations(json!([
                {"op":"clear","range":"A1"},
                {"op":"put","sheet":"Data","at":"A2","values":[[4]]}
            ]))?,
            &limits
        )?
        .total_cells,
        3
    );
    Ok(())
}

#[test]
fn named_range_count_is_bounded_but_replacing_one_name_is_allowed() -> TestResult {
    let limits = Limits {
        max_cells: 2,
        ..Limits::default()
    };
    let mut book = Workbook::new();
    book.apply(
        &operations(json!([
            {"op":"name","name":"FIRST","range":"A1"},
            {"op":"name","name":"SECOND","range":"A2"}
        ]))?,
        &limits,
    )?;
    let before = book.to_json(&limits)?;
    refused(
        book.apply(
            &operations(json!([
                {"op":"name","name":"THIRD","range":"A3"}
            ]))?,
            &limits,
        ),
        ErrorCode::Limit,
    )?;
    assert_eq!(book.to_json(&limits)?, before);
    book.apply(
        &operations(json!([
            {"op":"name","name":"first","range":"B1"}
        ]))?,
        &limits,
    )?;
    assert_eq!(book.names.len(), 2);
    assert_eq!(book.names["FIRST"].range, "B1");
    Ok(())
}

#[test]
fn matching_filtered_pages_obey_row_and_cell_counts_with_physical_offsets() -> TestResult {
    let book = filter_book()?;
    let limits = Limits {
        max_read_rows: 2,
        max_read_cells: 2,
        ..Limits::default()
    };
    let request = ReadRequest {
        columns: Some(vec!["C".into(), "A".into()]),
        limit: 2,
        filter: Some(Filter {
            column: "B".into(),
            op: FilterOp::Ge,
            value: Value::Number(0.0),
        }),
        ..read("A1:C3")
    };
    let first = book.read(&request, &limits)?;
    assert_eq!(
        first.rows,
        vec![vec![Value::Number(200.0), Value::Text("zero".into())]]
    );
    assert_eq!(first.row_numbers, vec![2]);
    assert_eq!(first.scanned_rows, 2);
    assert_eq!(first.next_offset, Some(2));
    let last = book.read(
        &ReadRequest {
            offset: 2,
            ..request.clone()
        },
        &limits,
    )?;
    assert_eq!(
        last.rows,
        vec![vec![Value::Number(300.0), Value::Text("two".into())]]
    );
    assert_eq!(last.row_numbers, vec![3]);
    assert_eq!(last.next_offset, None);
    for limit in [0, 3] {
        refused(
            book.read(
                &ReadRequest {
                    limit,
                    ..request.clone()
                },
                &limits,
            ),
            ErrorCode::Limit,
        )?;
    }
    refused(
        book.read(
            &ReadRequest {
                columns: Some(vec!["A".into(), "B".into(), "C".into()]),
                ..request
            },
            &limits,
        ),
        ErrorCode::Limit,
    )?;
    Ok(())
}
