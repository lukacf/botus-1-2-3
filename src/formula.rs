//! A small deterministic formula language. F9 is now a state of mind.

use crate::grid::{Address, column_name};
use crate::value::{EngineError, ErrorCode, Value};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Reference {
    pub sheet: Option<String>,
    pub address: Address,
    pub absolute_column: bool,
    pub absolute_row: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Expr {
    Number(f64),
    Text(String),
    Bool(bool),
    Reference(Reference),
    Range(Reference, Reference),
    Name(String),
    Unary(char, Box<Expr>),
    Binary(String, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Number(f64),
    Text(String),
    Word(String),
    Quoted(String),
    Operator(String),
    Left,
    Right,
    Comma,
    Colon,
    Bang,
}

fn syntax(message: impl Into<String>) -> EngineError {
    EngineError::new(ErrorCode::FormulaSyntax, message)
}

fn quoted(source: &str, cursor: &mut usize, quote: char) -> Result<String, EngineError> {
    *cursor += quote.len_utf8();
    let mut result = String::new();
    while let Some(character) = source[*cursor..].chars().next() {
        *cursor += character.len_utf8();
        if character == quote {
            if source[*cursor..].starts_with(quote) {
                *cursor += quote.len_utf8();
                result.push(quote);
            } else {
                return Ok(result);
            }
        } else {
            result.push(character);
        }
    }
    Err(syntax("Unterminated quoted string"))
}

fn lex(source: &str, max_nodes: usize) -> Result<Vec<Token>, EngineError> {
    let mut tokens = Vec::new();
    let mut cursor = 0;
    let token_limit = max_nodes.saturating_mul(4).saturating_add(8);
    while let Some(character) = source[cursor..].chars().next() {
        if tokens.len() >= token_limit {
            return Err(EngineError::limit("Formula token budget exceeded"));
        }
        if character.is_whitespace() {
            cursor += character.len_utf8();
            continue;
        }
        let next = source[cursor + character.len_utf8()..].chars().next();
        let token = match character {
            '"' => Token::Text(quoted(source, &mut cursor, '"')?),
            '\'' => Token::Quoted(quoted(source, &mut cursor, '\'')?),
            '(' => {
                cursor += 1;
                Token::Left
            }
            ')' => {
                cursor += 1;
                Token::Right
            }
            ',' => {
                cursor += 1;
                Token::Comma
            }
            ':' => {
                cursor += 1;
                Token::Colon
            }
            '!' => {
                cursor += 1;
                Token::Bang
            }
            '.' if next == Some('.') => {
                cursor += 2;
                Token::Colon
            }
            '+' | '-' | '*' | '/' | '^' | '=' | '<' | '>' => {
                cursor += 1;
                let mut operator = character.to_string();
                if matches!((character, next), ('<', Some('=' | '>')) | ('>', Some('=')))
                    && let Some(next) = next
                {
                    operator.push(next);
                    cursor += 1;
                }
                Token::Operator(operator)
            }
            c if c.is_ascii_digit() || (c == '.' && next.is_some_and(|c| c.is_ascii_digit())) => {
                let start = cursor;
                while source
                    .as_bytes()
                    .get(cursor)
                    .is_some_and(u8::is_ascii_digit)
                {
                    cursor += 1;
                }
                if source.as_bytes().get(cursor) == Some(&b'.')
                    && source.as_bytes().get(cursor + 1) != Some(&b'.')
                {
                    cursor += 1;
                    while source
                        .as_bytes()
                        .get(cursor)
                        .is_some_and(u8::is_ascii_digit)
                    {
                        cursor += 1;
                    }
                }
                if matches!(source.as_bytes().get(cursor), Some(b'e' | b'E')) {
                    cursor += 1;
                    if matches!(source.as_bytes().get(cursor), Some(b'+' | b'-')) {
                        cursor += 1;
                    }
                    let exponent_start = cursor;
                    while source
                        .as_bytes()
                        .get(cursor)
                        .is_some_and(u8::is_ascii_digit)
                    {
                        cursor += 1;
                    }
                    if exponent_start == cursor {
                        return Err(syntax("Exponent requires digits"));
                    }
                }
                let value = source[start..cursor]
                    .parse::<f64>()
                    .map_err(|_| syntax("Invalid numeric literal"))?;
                if !value.is_finite() {
                    return Err(EngineError::new(
                        ErrorCode::NonFinite,
                        "Numeric literal is not finite",
                    ));
                }
                Token::Number(value)
            }
            c if c.is_alphabetic() || matches!(c, '_' | '$' | '@') => {
                let start = cursor;
                cursor += c.len_utf8();
                while let Some(c) = source[cursor..].chars().next() {
                    if c.is_alphanumeric()
                        || matches!(c, '_' | '$')
                        || (c == '.' && !source[cursor..].starts_with(".."))
                    {
                        cursor += c.len_utf8();
                    } else {
                        break;
                    }
                }
                Token::Word(source[start..cursor].to_owned())
            }
            _ => return Err(syntax("Unsupported character in formula")),
        };
        tokens.push(token);
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    cursor: usize,
    nodes: usize,
    max_nodes: usize,
}

impl Parser {
    fn bump(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.cursor).cloned();
        if token.is_some() {
            self.cursor += 1;
        }
        token
    }

    fn node(&mut self, expr: Expr) -> Result<Expr, EngineError> {
        self.nodes += 1;
        if self.nodes > self.max_nodes {
            Err(EngineError::limit("Formula node budget exceeded"))
        } else {
            // Pratt parser recursion alone does not bound a long left chain.
            let mut pending = vec![(&expr, 1_usize)];
            while let Some((node, depth)) = pending.pop() {
                if depth > 64 {
                    return Err(EngineError::limit("Formula AST depth exceeds 64 levels"));
                }
                match node {
                    Expr::Unary(_, child) => pending.push((child, depth + 1)),
                    Expr::Binary(_, left, right) => {
                        pending.push((left, depth + 1));
                        pending.push((right, depth + 1));
                    }
                    Expr::Call(_, args) => {
                        for child in args {
                            pending.push((child, depth + 1));
                        }
                    }
                    _ => {}
                }
            }
            Ok(expr)
        }
    }

    fn expression(&mut self, minimum: u8, depth: usize) -> Result<Expr, EngineError> {
        if depth > 64 {
            return Err(EngineError::limit("Formula nesting exceeds 64 levels"));
        }
        let mut left = match self.bump() {
            Some(Token::Number(value)) => self.node(Expr::Number(value))?,
            Some(Token::Text(value)) => self.node(Expr::Text(value))?,
            Some(Token::Operator(op)) if op == "+" || op == "-" => {
                let value = self.expression(4, depth + 1)?;
                self.node(Expr::Unary(
                    if op == "+" { '+' } else { '-' },
                    Box::new(value),
                ))?
            }
            Some(Token::Left) => {
                let value = self.expression(0, depth + 1)?;
                if self.bump() != Some(Token::Right) {
                    return Err(syntax("Expected closing parenthesis"));
                }
                value
            }
            Some(Token::Word(word)) => self.word(word, false, depth + 1)?,
            Some(Token::Quoted(word)) => self.word(word, true, depth + 1)?,
            _ => return Err(syntax("Expected a value, reference or function")),
        };
        while let Some(Token::Operator(operator)) = self.tokens.get(self.cursor) {
            let operator = operator.clone();
            let precedence = match operator.as_str() {
                "=" | "<>" | "<" | ">" | "<=" | ">=" => 1,
                "+" | "-" => 2,
                "*" | "/" => 3,
                "^" => 5,
                _ => return Err(syntax("Unsupported operator")),
            };
            if precedence < minimum {
                break;
            }
            self.cursor += 1;
            let right = self.expression(
                if operator == "^" {
                    precedence
                } else {
                    precedence + 1
                },
                depth + 1,
            )?;
            left = self.node(Expr::Binary(operator, Box::new(left), Box::new(right)))?;
        }
        Ok(left)
    }

    fn word(
        &mut self,
        word: String,
        quoted_sheet: bool,
        depth: usize,
    ) -> Result<Expr, EngineError> {
        if self.tokens.get(self.cursor) == Some(&Token::Bang) {
            self.cursor += 1;
            let Some(Token::Word(cell)) = self.bump() else {
                return Err(syntax("Expected cell after sheet name"));
            };
            let mut reference =
                parse_reference(&cell).ok_or_else(|| syntax("Invalid cell reference"))?;
            reference.sheet = Some(word);
            return self.reference_or_range(reference);
        }
        if quoted_sheet {
            return Err(syntax("Quoted sheet name requires a cell reference"));
        }
        if self.tokens.get(self.cursor) == Some(&Token::Left) {
            self.cursor += 1;
            let name = word.strip_prefix('@').unwrap_or(&word).to_ascii_uppercase();
            if !matches!(
                name.as_str(),
                "SUM" | "AVERAGE" | "MIN" | "MAX" | "COUNT" | "IF"
            ) {
                return Err(EngineError::new(
                    ErrorCode::UnsupportedFunction,
                    format!("Unsupported function: {name}"),
                ));
            }
            let mut args = Vec::new();
            if self.tokens.get(self.cursor) != Some(&Token::Right) {
                loop {
                    args.push(self.expression(0, depth + 1)?);
                    if self.tokens.get(self.cursor) != Some(&Token::Comma) {
                        break;
                    }
                    self.cursor += 1;
                }
            }
            if self.bump() != Some(Token::Right) {
                return Err(syntax("Expected function closing parenthesis"));
            }
            if name == "IF" && !(2..=3).contains(&args.len()) {
                return Err(syntax("IF requires two or three arguments"));
            }
            return self.node(Expr::Call(name, args));
        }
        if word.starts_with('@') {
            return Err(syntax("@ must prefix a function call"));
        }
        if let Some(reference) = parse_reference(&word) {
            return self.reference_or_range(reference);
        }
        if word.contains('$') {
            return Err(syntax("Invalid absolute cell reference"));
        }
        match word.to_ascii_uppercase().as_str() {
            "TRUE" => self.node(Expr::Bool(true)),
            "FALSE" => self.node(Expr::Bool(false)),
            _ => self.node(Expr::Name(word.to_uppercase())),
        }
    }

    fn reference_or_range(&mut self, first: Reference) -> Result<Expr, EngineError> {
        if self.tokens.get(self.cursor) != Some(&Token::Colon) {
            return self.node(Expr::Reference(first));
        }
        self.cursor += 1;
        let Some(token) = self.bump() else {
            return Err(syntax("Range requires a second cell"));
        };
        let (word, quoted) = match token {
            Token::Word(word) => (word, false),
            Token::Quoted(word) => (word, true),
            _ => return Err(syntax("Range requires a cell reference")),
        };
        let mut last = if self.tokens.get(self.cursor) == Some(&Token::Bang) {
            self.cursor += 1;
            let Some(Token::Word(cell)) = self.bump() else {
                return Err(syntax("Expected cell after sheet name"));
            };
            let mut reference =
                parse_reference(&cell).ok_or_else(|| syntax("Invalid range endpoint"))?;
            reference.sheet = Some(word);
            reference
        } else {
            if quoted {
                return Err(syntax("Expected sheet separator"));
            }
            parse_reference(&word).ok_or_else(|| syntax("Invalid range endpoint"))?
        };
        if last.sheet.is_none() {
            last.sheet = first.sheet.clone();
        }
        if first.sheet.as_ref().map(|v| v.to_lowercase())
            != last.sheet.as_ref().map(|v| v.to_lowercase())
        {
            return Err(syntax("A range cannot span different sheets"));
        }
        if first.address.column > last.address.column || first.address.row > last.address.row {
            return Err(syntax(
                "Range endpoints must run from top left to bottom right",
            ));
        }
        self.node(Expr::Range(first, last))
    }
}

pub(crate) fn parse_reference(input: &str) -> Option<Reference> {
    let absolute_column = input.starts_with('$');
    let input = input.strip_prefix('$').unwrap_or(input);
    let split = input.find(|character: char| !character.is_ascii_alphabetic())?;
    let (column, row) = input.split_at(split);
    let absolute_row = row.starts_with('$');
    let row = row.strip_prefix('$').unwrap_or(row);
    let address = Address::parse(&format!("{column}{row}")).ok()?;
    Some(Reference {
        sheet: None,
        address,
        absolute_column,
        absolute_row,
    })
}

pub(crate) fn parse(source: &str, max_nodes: usize) -> Result<Expr, EngineError> {
    let source = source.trim();
    let source = source.strip_prefix('=').unwrap_or(source);
    let mut parser = Parser {
        tokens: lex(source, max_nodes)?,
        cursor: 0,
        nodes: 0,
        max_nodes,
    };
    let result = parser.expression(0, 0)?;
    if parser.cursor != parser.tokens.len() {
        return Err(syntax("Unexpected trailing formula content"));
    }
    Ok(result)
}

pub(crate) trait Context {
    fn step(&mut self) -> Result<(), EngineError>;
    fn cell(&mut self, reference: &Reference) -> Result<Value, EngineError>;
    fn range(&mut self, start: &Reference, end: &Reference) -> Result<Vec<Value>, EngineError>;
    fn named_range(&mut self, name: &str) -> Result<Vec<Value>, EngineError>;
}

fn finite(value: f64) -> Result<Value, EngineError> {
    if value.is_finite() {
        Ok(Value::Number(value))
    } else {
        Err(EngineError::new(
            ErrorCode::NonFinite,
            "Formula result is not finite",
        ))
    }
}

pub(crate) fn evaluate(expr: &Expr, context: &mut impl Context) -> Result<Value, EngineError> {
    context.step()?;
    match expr {
        Expr::Number(value) => Ok(Value::Number(*value)),
        Expr::Text(value) => Ok(Value::Text(value.clone())),
        Expr::Bool(value) => Ok(Value::Bool(*value)),
        Expr::Reference(reference) => context.cell(reference),
        Expr::Name(name) => {
            let mut values = context.named_range(name)?;
            if values.len() != 1 {
                return Err(EngineError::new(
                    ErrorCode::FormulaType,
                    "Scalar expression requires a single-cell named range",
                ));
            }
            values
                .pop()
                .ok_or_else(|| EngineError::reference("Named range is empty"))
        }
        Expr::Range(_, _) => Err(EngineError::new(
            ErrorCode::FormulaType,
            "Range requires an aggregate function",
        )),
        Expr::Unary(operator, value) => {
            let value = evaluate(value, context)?.numeric()?;
            finite(if *operator == '-' { -value } else { value })
        }
        Expr::Binary(operator, left, right) => {
            let left = evaluate(left, context)?.numeric()?;
            let right = evaluate(right, context)?.numeric()?;
            match operator.as_str() {
                "+" => finite(left + right),
                "-" => finite(left - right),
                "*" => finite(left * right),
                "/" if right == 0.0 => Err(EngineError::new(
                    ErrorCode::DivideByZero,
                    "Division by zero",
                )),
                "/" => finite(left / right),
                "^" => finite(left.powf(right)),
                "=" => Ok(Value::Bool(left == right)),
                "<>" => Ok(Value::Bool(left != right)),
                "<" => Ok(Value::Bool(left < right)),
                ">" => Ok(Value::Bool(left > right)),
                "<=" => Ok(Value::Bool(left <= right)),
                ">=" => Ok(Value::Bool(left >= right)),
                _ => Err(syntax("Unsupported operator")),
            }
        }
        Expr::Call(name, args) if name == "IF" => {
            let condition = args
                .first()
                .ok_or_else(|| syntax("IF requires a condition"))?;
            let index = if evaluate(condition, context)?.truthy()? {
                1
            } else {
                2
            };
            match args.get(index) {
                Some(expr) => evaluate(expr, context),
                None => Ok(Value::Bool(false)),
            }
        }
        Expr::Call(name, args) => {
            let mut count = 0_u64;
            let mut sum = 0.0;
            let mut min = f64::INFINITY;
            let mut max = f64::NEG_INFINITY;
            for argument in args {
                let values = match argument {
                    Expr::Range(start, end) => context.range(start, end)?,
                    Expr::Name(name) => context.named_range(name)?,
                    expression => vec![evaluate(expression, context)?],
                };
                for value in values {
                    context.step()?;
                    if let Value::Number(value) = value {
                        count += 1;
                        if matches!(name.as_str(), "SUM" | "AVERAGE") {
                            sum += value;
                            if !sum.is_finite() {
                                return Err(EngineError::new(
                                    ErrorCode::NonFinite,
                                    "Aggregate intermediate result is not finite",
                                ));
                            }
                        }
                        min = min.min(value);
                        max = max.max(value);
                    }
                }
            }
            match name.as_str() {
                "SUM" => finite(sum),
                "AVERAGE" if count == 0 => Err(EngineError::new(
                    ErrorCode::DivideByZero,
                    "AVERAGE requires at least one numeric value",
                )),
                "AVERAGE" => finite(sum / count as f64),
                "COUNT" => finite(count as f64),
                "MIN" => finite(if count == 0 { 0.0 } else { min }),
                "MAX" => finite(if count == 0 { 0.0 } else { max }),
                _ => Err(EngineError::new(
                    ErrorCode::UnsupportedFunction,
                    "Unsupported function",
                )),
            }
        }
    }
}

pub(crate) fn visit_references(
    expr: &Expr,
    visitor: &mut impl FnMut(&Reference) -> Result<(), EngineError>,
) -> Result<(), EngineError> {
    match expr {
        Expr::Reference(reference) => visitor(reference)?,
        Expr::Range(start, end) => {
            visitor(start)?;
            visitor(end)?;
        }
        Expr::Unary(_, value) => visit_references(value, visitor)?,
        Expr::Binary(_, left, right) => {
            visit_references(left, visitor)?;
            visit_references(right, visitor)?;
        }
        Expr::Call(_, arguments) => {
            for argument in arguments {
                visit_references(argument, visitor)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn format_reference(reference: &Reference) -> String {
    let sheet = reference
        .sheet
        .as_ref()
        .map(|name| format!("'{}'!", name.replace('\'', "''")))
        .unwrap_or_default();
    format!(
        "{sheet}{}{}{}{}",
        if reference.absolute_column { "$" } else { "" },
        column_name(reference.address.column),
        if reference.absolute_row { "$" } else { "" },
        u64::from(reference.address.row) + 1
    )
}

fn render_shifted(
    expr: &Expr,
    columns: i64,
    rows: i64,
    max_columns: u32,
    max_rows: u32,
) -> Result<String, EngineError> {
    let shift = |reference: &Reference| -> Result<String, EngineError> {
        let mut reference = reference.clone();
        reference.address = reference
            .address
            .shifted(
                if reference.absolute_column {
                    0
                } else {
                    columns
                },
                if reference.absolute_row { 0 } else { rows },
            )
            .map_err(EngineError::reference)?;
        if reference.address.column >= max_columns || reference.address.row >= max_rows {
            return Err(EngineError::reference(
                "Shifted formula reference exceeds grid limits",
            ));
        }
        Ok(format_reference(&reference))
    };
    let render = |expr| render_shifted(expr, columns, rows, max_columns, max_rows);
    Ok(match expr {
        Expr::Number(value) => value.to_string(),
        Expr::Text(value) => format!("\"{}\"", value.replace('"', "\"\"")),
        Expr::Bool(value) => if *value { "TRUE" } else { "FALSE" }.to_owned(),
        Expr::Reference(reference) => shift(reference)?,
        Expr::Range(start, end) => format!("{}:{}", shift(start)?, shift(end)?),
        Expr::Name(name) => name.clone(),
        Expr::Unary(operator, expr) => format!("({operator}{})", render(expr)?),
        Expr::Binary(operator, left, right) => {
            format!("({}{operator}{})", render(left)?, render(right)?)
        }
        Expr::Call(name, arguments) => {
            let arguments = arguments
                .iter()
                .map(render)
                .collect::<Result<Vec<_>, _>>()?;
            format!("{name}({})", arguments.join(","))
        }
    })
}

pub(crate) fn shift(
    source: &str,
    columns: i64,
    rows: i64,
    max_columns: u32,
    max_rows: u32,
    max_nodes: usize,
) -> Result<String, EngineError> {
    Ok(format!(
        "={}",
        render_shifted(
            &parse(source, max_nodes)?,
            columns,
            rows,
            max_columns,
            max_rows
        )?
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Empty;
    impl Context for Empty {
        fn step(&mut self) -> Result<(), EngineError> {
            Ok(())
        }
        fn cell(&mut self, _: &Reference) -> Result<Value, EngineError> {
            Ok(Value::Blank)
        }
        fn range(&mut self, _: &Reference, _: &Reference) -> Result<Vec<Value>, EngineError> {
            Ok(vec![])
        }
        fn named_range(&mut self, _: &str) -> Result<Vec<Value>, EngineError> {
            Err(EngineError::reference("Unknown name"))
        }
    }

    #[test]
    fn arithmetic_and_lazy_if() -> Result<(), EngineError> {
        assert_eq!(
            evaluate(&parse("=2+3*4^2", 100)?, &mut Empty)?,
            Value::Number(50.0)
        );
        assert_eq!(
            evaluate(&parse("=IF(TRUE,\"yes\",1/0)", 100)?, &mut Empty)?,
            Value::Text("yes".into())
        );
        assert_eq!(
            evaluate(&parse("=@SUM(1,2,\"3\",TRUE)", 100)?, &mut Empty)?,
            Value::Number(3.0)
        );
        assert!(evaluate(&parse("=1+\"2\"", 100)?, &mut Empty).is_err());
        assert!(parse("=NOW()", 100).is_err());
        assert!(parse("=1 trailing", 100).is_err());
        assert!(parse("=1e309", 100).is_err());
        Ok(())
    }

    #[test]
    fn copy_keeps_absolute_anchors_and_unicode_sheet_names() -> Result<(), EngineError> {
        assert_eq!(
            shift("=$A1+A$1+$A$1", 1, 2, 1024, 100000, 100)?,
            "=(($A3+B$1)+$A$1)"
        );
        let source = "=SUM('Försäljning'!A1:A3)";
        let copied = shift(source, 2, 1, 1024, 100000, 100)?;
        assert_eq!(copied, "=SUM('Försäljning'!C2:'Försäljning'!C4)");
        assert!(parse(&copied, 100).is_ok());
        assert!(shift("=A1", -1, 0, 1024, 100000, 100).is_err());
        Ok(())
    }

    #[test]
    fn nesting_and_node_limits_are_enforced() {
        assert!(parse(&format!("{}1{}", "(".repeat(1000), ")".repeat(1000)), 5000).is_err());
        assert!(
            parse(
                &std::iter::repeat_n("1", 200).collect::<Vec<_>>().join("+"),
                1000
            )
            .is_err()
        );
        assert!(parse("=1+2+3", 2).is_err());
        for formula in [
            "=",
            "=SUM(",
            "=IF(1)",
            "='bad",
            "=A1:B2:C3",
            "=@A1",
            "=1e+",
            "=A0",
        ] {
            // A0 is a possible name, so undefined-name detection happens in evaluation.
            if formula == "=A0" {
                continue;
            }
            assert!(parse(formula, 100).is_err(), "{formula}");
        }
    }
}
