//! A1 coordinates, with considerably more elbow room than a floppy label.

use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct Address {
    pub column: u32,
    pub row: u32,
}

impl Address {
    pub fn parse(input: &str) -> Result<Self, String> {
        let split = input
            .find(|c: char| !c.is_ascii_alphabetic())
            .ok_or_else(|| "Cell address requires a row number".to_owned())?;
        if split == 0 {
            return Err("Cell address requires a column".to_owned());
        }
        let column = parse_column(&input[..split])?;
        let digits = &input[split..];
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err("Cell row must be a positive integer".to_owned());
        }
        let row = digits
            .parse::<u32>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .ok_or_else(|| "Cell row is out of bounds".to_owned())?;
        Ok(Self { column, row })
    }

    pub fn shifted(self, columns: i64, rows: i64) -> Result<Self, String> {
        let column = i64::from(self.column)
            .checked_add(columns)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| "Shifted column is out of bounds".to_owned())?;
        let row = i64::from(self.row)
            .checked_add(rows)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| "Shifted row is out of bounds".to_owned())?;
        Ok(Self { column, row })
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", column_name(self.column), u64::from(self.row) + 1)
    }
}

pub(crate) fn parse_column(input: &str) -> Result<u32, String> {
    if input.is_empty() {
        return Err("Column cannot be empty".to_owned());
    }
    let mut value = 0_u32;
    for byte in input.bytes() {
        if !byte.is_ascii_alphabetic() {
            return Err("Column must contain only ASCII letters".to_owned());
        }
        value = value
            .checked_mul(26)
            .and_then(|n| n.checked_add(u32::from(byte.to_ascii_uppercase() - b'A') + 1))
            .ok_or_else(|| "Column is out of bounds".to_owned())?;
    }
    value
        .checked_sub(1)
        .ok_or_else(|| "Column is out of bounds".to_owned())
}

pub(crate) fn column_name(column: u32) -> String {
    let mut value = u64::from(column) + 1;
    let mut chars = Vec::new();
    while value > 0 {
        chars.push(char::from(b'A' + ((value - 1) % 26) as u8));
        value = (value - 1) / 26;
    }
    chars.into_iter().rev().collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct GridRange {
    pub start: Address,
    pub end: Address,
}

impl GridRange {
    pub fn parse(input: &str) -> Result<Self, String> {
        let parts: Vec<&str> = if input.contains("..") {
            input.split("..").collect()
        } else {
            input.split(':').collect()
        };
        let (start, end) = match parts.as_slice() {
            [cell] => {
                let address = Address::parse(cell)?;
                (address, address)
            }
            [start, end] => (Address::parse(start)?, Address::parse(end)?),
            _ => return Err("Range requires one cell or two endpoints".to_owned()),
        };
        if start.column > end.column || start.row > end.row {
            return Err("Range endpoints must run from top left to bottom right".to_owned());
        }
        Ok(Self { start, end })
    }

    pub fn width(self) -> u64 {
        u64::from(self.end.column) - u64::from(self.start.column) + 1
    }

    pub fn height(self) -> u64 {
        u64::from(self.end.row) - u64::from(self.start.row) + 1
    }

    pub fn area(self) -> Option<u64> {
        self.width().checked_mul(self.height())
    }

    pub fn contains(self, address: Address) -> bool {
        address.column >= self.start.column
            && address.column <= self.end.column
            && address.row >= self.start.row
            && address.row <= self.end.row
    }

    pub fn include(&mut self, address: Address) {
        self.start.column = self.start.column.min(address.column);
        self.start.row = self.start.row.min(address.row);
        self.end.column = self.end.column.max(address.column);
        self.end.row = self.end.row.max(address.row);
    }
}

impl fmt::Display for GridRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.start == self.end {
            self.start.fmt(f)
        } else {
            write!(f, "{}:{}", self.start, self.end)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinates_round_trip_without_overflow() -> Result<(), String> {
        for column in [0, 25, 26, 255, 1023, u32::MAX - 1] {
            let cell = Address { column, row: 8191 };
            assert_eq!(Address::parse(&cell.to_string())?, cell);
        }
        assert!(Address::parse("A0").is_err());
        assert!(Address::parse("A4294967296").is_err());
        assert!(GridRange::parse("B2:A1").is_err());
        assert!(GridRange::parse("A1:B2:C3").is_err());
        assert_eq!(GridRange::parse("a1..c4")?.to_string(), "A1:C4");
        Ok(())
    }
}
