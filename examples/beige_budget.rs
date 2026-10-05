//! The beige box is optional. The arithmetic is not.
use botus_1_2_3::{Limits, Operation, ReadRequest, Workbook};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let limits = Limits::default();
    let mut book = Workbook::new();
    let operations: Vec<Operation> = serde_json::from_value(serde_json::json!([
        {"op":"put","at":"A1","values":[["Item","Count","Cost"],["Emergency floppy disks",3,4],["F9 key insurance",1,12]]},
        {"op":"formula","at":"D2","formula":"=B2*C2"},
        {"op":"fill","range":"D2","to":"D2:D3"},
        {"op":"formula","at":"D4","formula":"=@SUM(D2:D3)"}
    ]))?;
    book.apply(&operations, &limits)?;
    let result = book.read(
        &ReadRequest {
            range: Some("A1:D4".into()),
            ..ReadRequest::default()
        },
        &limits,
    )?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
