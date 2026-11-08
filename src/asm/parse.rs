//! Parsing lines of Thumb assembly into statements.

use super::ErrorKind;
use crate::memory;
use std::collections::HashMap;

/// One term of an expression: a number or the name of a label or constant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Term {
    /// A number.
    Number(i64),
    /// The name of a label or constant.
    Symbol(String),
}

/// A sum of terms, each added or subtracted, such as `HOOK+1`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expr(pub Vec<(bool, Term)>);

/// One operand of an instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operand {
    /// A register, such as `r0` or `sp`.
    Reg(u8),
    /// A register written back after a multiple transfer, such as `r0!`.
    WriteBack(u8),
    /// An immediate value, such as `#16`.
    Imm(Expr),
    /// A value loaded from the literal pool, such as `=0x02000348`.
    Literal(Expr),
    /// A memory address, such as `[r0, #4]` or `[r0, r1]`.
    Mem {
        /// The base register.
        base: u8,
        /// The offset added to it, `#0` if none is given.
        offset: Offset,
    },
    /// A register list, as a bit for each register, such as `{r4-r6, lr}`.
    List(u16),
    /// An address, such as a branch target.
    Address(Expr),
}

/// The offset of a memory operand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Offset {
    /// An immediate offset, such as `#4`.
    Imm(Expr),
    /// A register offset.
    Reg(u8),
}

/// A parsed line of assembly.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Line {
    /// The label naming the line's address, if any.
    pub label: Option<String>,
    /// The directive or instruction after the label, if any.
    pub statement: Option<Statement>,
    /// The text of the statement, without the label or comment.
    pub text: String,
}

/// A directive or instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Statement {
    /// A `.equ` constant.
    Equ(String, Expr),
    /// Values of `size` bytes each, from `.byte`, `.hword`, or `.word`.
    Data {
        /// The size of each value in bytes.
        size: u8,
        /// The values.
        values: Vec<Expr>,
    },
    /// An `.align` to a multiple of 2 to this power.
    Align(u32),
    /// An instruction.
    Instruction {
        /// The mnemonic, in lowercase.
        mnemonic: String,
        /// The operands.
        operands: Vec<Operand>,
    },
}

/// The largest power of 2 that `.align` takes.
pub const MAX_ALIGN: u32 = 4;

impl Expr {
    /// Returns the value of the expression, looking up each name in
    /// `symbols`.
    pub fn eval(&self, symbols: &HashMap<String, i64>) -> Result<i64, ErrorKind> {
        self.0.iter().try_fold(0, |sum, (negative, term)| {
            let value = match term {
                Term::Number(number) => *number,
                Term::Symbol(name) => *symbols
                    .get(name)
                    .ok_or_else(|| ErrorKind::Undefined(name.clone()))?,
            };
            Ok(if *negative { sum - value } else { sum + value })
        })
    }
}

/// Parses one line: a label, followed by a directive or an instruction, each
/// of them optional. A comment starts with `@`.
pub fn line(text: &str) -> Result<Line, ErrorKind> {
    let mut text = text.split_once('@').map_or(text, |(code, _)| code).trim();
    let mut line = Line::default();
    if let Some((name, rest)) = text.split_once(':') {
        let name = name.trim();
        if !is_symbol(name) {
            return Err(ErrorKind::InvalidLabel(name.to_string()));
        }
        line.label = Some(name.to_string());
        text = rest.trim();
    }
    if text.is_empty() {
        return Ok(line);
    }
    let (word, rest) = text
        .split_once(char::is_whitespace)
        .map_or((text, ""), |(word, rest)| (word, rest.trim()));
    let word = word.to_lowercase();
    let statement = if word.starts_with('.') {
        directive(&word, rest)?
    } else {
        Statement::Instruction {
            operands: split(rest)
                .into_iter()
                .map(operand)
                .collect::<Result<_, _>>()?,
            mnemonic: word,
        }
    };
    line.statement = Some(statement);
    line.text = text.to_string();
    Ok(line)
}

/// Parses a directive's arguments.
fn directive(name: &str, args: &str) -> Result<Statement, ErrorKind> {
    let data = |size| {
        let values = split(args)
            .into_iter()
            .map(expr)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Statement::Data { size, values })
    };
    match name {
        ".byte" => data(1),
        ".hword" => data(2),
        ".word" => data(4),
        ".align" => {
            let power = memory::parse_number(args)
                .ok()
                .filter(|&power| power <= MAX_ALIGN)
                .ok_or_else(|| ErrorKind::InvalidAlign(args.to_string()))?;
            Ok(Statement::Align(power))
        }
        ".equ" => match split(args).as_slice() {
            [name, value] if is_symbol(name) => Ok(Statement::Equ(name.to_string(), expr(value)?)),
            _ => Err(ErrorKind::InvalidEqu(args.to_string())),
        },
        _ => Err(ErrorKind::Unknown(name.to_string())),
    }
}

/// Splits text at the commas that aren't inside brackets or braces, trimming
/// each part. Empty text has no parts.
fn split(text: &str) -> Vec<&str> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    let mut parts = Vec::new();
    let mut depth = 0_i32;
    let mut start = 0;
    for (index, char) in text.char_indices() {
        match char {
            '[' | '{' => depth += 1,
            ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(text[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(text[start..].trim());
    parts
}

/// Parses one operand.
fn operand(text: &str) -> Result<Operand, ErrorKind> {
    let invalid = || ErrorKind::InvalidOperand(text.to_string());
    if let Some(value) = text.strip_prefix('#') {
        return Ok(Operand::Imm(expr(value)?));
    }
    if let Some(value) = text.strip_prefix('=') {
        return Ok(Operand::Literal(expr(value)?));
    }
    if let Some(inner) = text
        .strip_prefix('[')
        .and_then(|text| text.strip_suffix(']'))
    {
        let offset = |text: &str| {
            text.strip_prefix('#').map_or_else(
                || register(text).map(Offset::Reg).ok_or_else(invalid),
                |value| expr(value).map(Offset::Imm),
            )
        };
        return match split(inner).as_slice() {
            [base] => Ok(Operand::Mem {
                base: register(base).ok_or_else(invalid)?,
                offset: Offset::Imm(Expr(vec![(false, Term::Number(0))])),
            }),
            [base, value] => Ok(Operand::Mem {
                base: register(base).ok_or_else(invalid)?,
                offset: offset(value)?,
            }),
            _ => Err(invalid()),
        };
    }
    if let Some(inner) = text
        .strip_prefix('{')
        .and_then(|text| text.strip_suffix('}'))
    {
        return list(inner).ok_or_else(invalid).map(Operand::List);
    }
    if let Some(name) = text.strip_suffix('!') {
        return register(name).map(Operand::WriteBack).ok_or_else(invalid);
    }
    if let Some(reg) = register(text) {
        return Ok(Operand::Reg(reg));
    }
    expr(text).map(Operand::Address)
}

/// Parses a register list's contents, such as `r4-r6, lr`.
fn list(text: &str) -> Option<u16> {
    split(text).into_iter().try_fold(0_u16, |bits, item| {
        let (first, last) = match item.split_once('-') {
            Some((first, last)) => (register(first)?, register(last)?),
            None => (register(item)?, register(item)?),
        };
        (first <= last).then(|| (first..=last).fold(bits, |bits, reg| bits | 1 << reg))
    })
}

/// Returns the number of a register, from `r0` to `r15`, or its other names
/// `sb`, `sl`, `fp`, `ip`, `sp`, `lr`, and `pc`.
pub fn register(text: &str) -> Option<u8> {
    let text = text.trim().to_lowercase();
    let named = match text.as_str() {
        "sb" => Some(9),
        "sl" => Some(10),
        "fp" => Some(11),
        "ip" => Some(12),
        "sp" => Some(13),
        "lr" => Some(14),
        "pc" => Some(15),
        _ => None,
    };
    named.or_else(|| {
        let number = text.strip_prefix('r')?;
        let reg: u8 = number.parse().ok()?;
        (reg < 16 && number == reg.to_string()).then_some(reg)
    })
}

/// Parses an expression: numbers and names, each added or subtracted.
pub fn expr(text: &str) -> Result<Expr, ErrorKind> {
    let text = text.trim();
    let invalid = || ErrorKind::InvalidValue(text.to_string());
    let mut terms = Vec::new();
    let (mut negative, mut rest) = text
        .strip_prefix('-')
        .map_or((false, text), |rest| (true, rest));
    loop {
        let end = rest.find(['+', '-']).unwrap_or(rest.len());
        terms.push((negative, term(rest[..end].trim()).ok_or_else(invalid)?));
        let Some(operator) = rest[end..].chars().next() else {
            return Ok(Expr(terms));
        };
        negative = operator == '-';
        rest = &rest[end + 1..];
    }
}

/// Parses a number or a name.
fn term(text: &str) -> Option<Term> {
    if text.starts_with(|char: char| char.is_ascii_digit()) {
        memory::parse_number(text)
            .ok()
            .map(|number| Term::Number(number.into()))
    } else {
        is_symbol(text).then(|| Term::Symbol(text.to_string()))
    }
}

/// Returns whether the text is a valid name: a letter or underscore,
/// followed by letters, digits, or underscores.
fn is_symbol(text: &str) -> bool {
    text.starts_with(|char: char| char.is_ascii_alphabetic() || char == '_')
        && text
            .chars()
            .all(|char| char.is_ascii_alphanumeric() || char == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn number(value: i64) -> Expr {
        Expr(vec![(false, Term::Number(value))])
    }

    fn symbol(name: &str) -> Expr {
        Expr(vec![(false, Term::Symbol(name.into()))])
    }

    fn instruction(mnemonic: &str, operands: Vec<Operand>) -> Statement {
        Statement::Instruction {
            mnemonic: mnemonic.into(),
            operands,
        }
    }

    #[test]
    fn parse_expressions() {
        assert_eq!(expr("0x10"), Ok(number(16)));
        assert_eq!(expr(" -4 "), Ok(Expr(vec![(true, Term::Number(4))])));
        assert_eq!(
            expr("HOOK + 1 - _base2"),
            Ok(Expr(vec![
                (false, Term::Symbol("HOOK".into())),
                (false, Term::Number(1)),
                (true, Term::Symbol("_base2".into())),
            ]))
        );
        for text in ["", "1+", "--1", "0xZZ", "9a", "a.b", "1 2"] {
            assert_eq!(expr(text), Err(ErrorKind::InvalidValue(text.trim().into())));
        }
    }

    #[test]
    fn evaluate_expressions() {
        let symbols = HashMap::from([("HOOK".to_string(), 0x0800_0000)]);
        assert_eq!(expr("HOOK+1").unwrap().eval(&symbols), Ok(0x0800_0001));
        assert_eq!(expr("4-HOOK").unwrap().eval(&symbols), Ok(4 - 0x0800_0000));
        assert_eq!(
            expr("MISSING").unwrap().eval(&symbols),
            Err(ErrorKind::Undefined("MISSING".into()))
        );
    }

    #[test]
    fn parse_registers() {
        for (text, reg) in [
            ("r0", 0),
            ("R7", 7),
            ("r15", 15),
            ("sb", 9),
            ("sl", 10),
            ("fp", 11),
            ("ip", 12),
            ("SP", 13),
            ("lr", 14),
            ("pc", 15),
        ] {
            assert_eq!(register(text), Some(reg));
        }
        for text in ["r16", "r", "r01", "r+1", "x0", "r-1"] {
            assert_eq!(register(text), None);
        }
    }

    #[test]
    fn parse_operands() {
        assert_eq!(operand("r4"), Ok(Operand::Reg(4)));
        assert_eq!(operand("r0!"), Ok(Operand::WriteBack(0)));
        assert_eq!(operand("#180"), Ok(Operand::Imm(number(180))));
        assert_eq!(operand("=FN"), Ok(Operand::Literal(symbol("FN"))));
        assert_eq!(operand("done"), Ok(Operand::Address(symbol("done"))));
        assert_eq!(
            operand("[r1]"),
            Ok(Operand::Mem {
                base: 1,
                offset: Offset::Imm(number(0))
            })
        );
        assert_eq!(
            operand("[sp, #180]"),
            Ok(Operand::Mem {
                base: 13,
                offset: Offset::Imm(number(180))
            })
        );
        assert_eq!(
            operand("[r1, r2]"),
            Ok(Operand::Mem {
                base: 1,
                offset: Offset::Reg(2)
            })
        );
        assert_eq!(operand("{r4-r6, lr}"), Ok(Operand::List(0x4070)));
        assert_eq!(operand("{r0,r2-r3,pc}"), Ok(Operand::List(0x800D)));
        assert_eq!(operand("{}"), Ok(Operand::List(0)));
        for text in [
            "[x1]",
            "[r1, x2]",
            "[r1, r2, r3]",
            "[x1, r2]",
            "{r1-x2}",
            "{x1-r2}",
            "{r3-r1}",
            "{x1}",
            "x0!",
        ] {
            assert_eq!(operand(text), Err(ErrorKind::InvalidOperand(text.into())));
        }
        assert_eq!(
            operand("[r1, #x.y]"),
            Err(ErrorKind::InvalidValue("x.y".into()))
        );
    }

    /// Parses a line, returning only its statement.
    fn statement(text: &str) -> Result<Option<Statement>, ErrorKind> {
        line(text).map(|line| line.statement)
    }

    #[test]
    fn parse_lines() {
        assert_eq!(line("   @ a comment"), Ok(Line::default()));
        assert_eq!(
            line("done:"),
            Ok(Line {
                label: Some("done".into()),
                ..Line::default()
            })
        );
        assert_eq!(
            line("  done: POP {r4}  @ restore"),
            Ok(Line {
                label: Some("done".into()),
                statement: Some(instruction("pop", vec![Operand::List(0x10)])),
                text: "POP {r4}".into(),
            })
        );
        assert_eq!(
            statement("ldr r2, [sp, #180]"),
            Ok(Some(instruction(
                "ldr",
                vec![
                    Operand::Reg(2),
                    Operand::Mem {
                        base: 13,
                        offset: Offset::Imm(number(180))
                    }
                ]
            )))
        );
        assert_eq!(statement("nop"), Ok(Some(instruction("nop", vec![]))));
        assert_eq!(
            statement("1abel: nop"),
            Err(ErrorKind::InvalidLabel("1abel".into()))
        );
        assert_eq!(
            statement("mov r0, 9z"),
            Err(ErrorKind::InvalidValue("9z".into()))
        );
    }

    #[test]
    fn parse_directives() {
        assert_eq!(
            statement(".word FN, 0x118"),
            Ok(Some(Statement::Data {
                size: 4,
                values: vec![symbol("FN"), number(0x118)]
            }))
        );
        assert_eq!(
            statement(".HWORD 1"),
            Ok(Some(Statement::Data {
                size: 2,
                values: vec![number(1)]
            }))
        );
        assert_eq!(
            statement(".byte 1, 2"),
            Ok(Some(Statement::Data {
                size: 1,
                values: vec![number(1), number(2)]
            }))
        );
        assert_eq!(
            statement(".byte 1,"),
            Err(ErrorKind::InvalidValue(String::new()))
        );
        assert_eq!(statement(".align 2"), Ok(Some(Statement::Align(2))));
        for args in ["9", "x", ""] {
            assert_eq!(
                statement(&format!(".align {args}")),
                Err(ErrorKind::InvalidAlign(args.into()))
            );
        }
        assert_eq!(
            statement(".equ FN, 0x08006A79"),
            Ok(Some(Statement::Equ("FN".into(), number(0x0800_6A79))))
        );
        for args in ["FN", "1x, 2", "FN, 1, 2"] {
            assert_eq!(
                statement(&format!(".equ {args}")),
                Err(ErrorKind::InvalidEqu(args.into()))
            );
        }
        assert_eq!(
            statement(".equ FN, 1x"),
            Err(ErrorKind::InvalidValue("1x".into()))
        );
        assert_eq!(statement(".pool"), Err(ErrorKind::Unknown(".pool".into())));
    }
}
