//! Encoding Thumb instructions.

// Values are range-checked before they're narrowed into an encoding.
#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]

use super::ErrorKind;
use super::parse::{Expr, Offset, Operand};
use std::collections::HashMap;

/// What an instruction's operands are resolved against.
pub struct Context<'a> {
    /// The address of the instruction.
    pub address: i64,
    /// The address of each label and the value of each constant.
    pub symbols: &'a HashMap<String, i64>,
    /// The address in the literal pool of each value loaded with `=`.
    pub literals: &'a HashMap<u32, i64>,
}

/// The mnemonics other than conditional branches and data operations on two
/// low registers.
const MNEMONICS: &[&str] = &[
    "add", "asr", "b", "bl", "bx", "cmp", "ldmia", "ldr", "ldrb", "ldrh", "ldrsb", "ldrsh", "ldsb",
    "ldsh", "lsl", "lsr", "mov", "nop", "pop", "push", "stmdb", "stmia", "str", "strb", "strh",
    "sub", "swi",
];

/// The largest offset of a load relative to `pc` or `sp`.
const MAX_WORD_OFFSET: i64 = 1020;

/// Converts a value to the 32-bit word it's stored as, either signed or
/// unsigned.
///
/// # Errors
///
/// Returns [`ErrorKind::Range`] if it fits neither way.
pub fn word(value: i64, what: &'static str) -> Result<u32, ErrorKind> {
    check_range(value, what, i64::from(i32::MIN), i64::from(u32::MAX))?;
    Ok(value as u32)
}

/// Checks that a value is from `min` to `max`.
pub fn check_range(value: i64, what: &'static str, min: i64, max: i64) -> Result<(), ErrorKind> {
    if (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(ErrorKind::Range {
            what,
            value,
            min,
            max,
        })
    }
}

impl Context<'_> {
    fn value(&self, expr: &Expr) -> Result<i64, ErrorKind> {
        expr.eval(self.symbols)
    }

    /// Returns a value checked to be from 0 to `max` and a multiple of
    /// `scale`, divided by `scale`.
    fn unsigned(
        &self,
        expr: &Expr,
        what: &'static str,
        max: i64,
        scale: i64,
    ) -> Result<u16, ErrorKind> {
        let value = self.value(expr)?;
        check_range(value, what, 0, max)?;
        check_multiple(value, what, scale)?;
        Ok((value / scale) as u16)
    }

    /// Returns the offset from the instruction's `pc` to a branch target,
    /// checked to be from `min` to `max`.
    fn branch(&self, target: &Expr, min: i64, max: i64) -> Result<i64, ErrorKind> {
        let target = word(self.value(target)?, "the target")?;
        if !target.is_multiple_of(2) {
            return Err(ErrorKind::OddTarget(target));
        }
        reach(target, i64::from(target) - (self.address + 4), min, max)
    }

    /// Encodes a load from `target`, relative to the instruction's `pc`
    /// rounded down to a word.
    fn load(&self, reg: u8, target: i64) -> Result<Vec<u16>, ErrorKind> {
        let target = word(target, "the target")?;
        let offset = i64::from(target) - ((self.address + 4) & !3);
        check_multiple(offset, "the offset", 4)?;
        let offset = reach(target, offset, 0, MAX_WORD_OFFSET)?;
        Ok(vec![0x4800 | r(reg) << 8 | (offset / 4) as u16])
    }
}

const fn check_multiple(value: i64, what: &'static str, multiple: i64) -> Result<(), ErrorKind> {
    if value % multiple == 0 {
        Ok(())
    } else {
        Err(ErrorKind::Unaligned {
            what,
            value,
            multiple,
        })
    }
}

fn reach(target: u32, offset: i64, min: i64, max: i64) -> Result<i64, ErrorKind> {
    if (min..=max).contains(&offset) {
        Ok(offset)
    } else {
        Err(ErrorKind::Reach {
            target,
            offset,
            min,
            max,
        })
    }
}

const fn low(reg: u8) -> bool {
    reg < 8
}

fn r(reg: u8) -> u16 {
    u16::from(reg)
}

/// Returns the condition code of a conditional branch.
fn condition(mnemonic: &str) -> Option<u16> {
    let code = match mnemonic {
        "beq" => 0,
        "bne" => 1,
        "bcs" | "bhs" => 2,
        "bcc" | "blo" => 3,
        "bmi" => 4,
        "bpl" => 5,
        "bvs" => 6,
        "bvc" => 7,
        "bhi" => 8,
        "bls" => 9,
        "bge" => 10,
        "blt" => 11,
        "bgt" => 12,
        "ble" => 13,
        _ => return None,
    };
    Some(code)
}

/// Returns the operation of a data instruction on two low registers.
fn alu(mnemonic: &str) -> Option<u16> {
    let op = match mnemonic {
        "and" => 0,
        "eor" => 1,
        "lsl" => 2,
        "lsr" => 3,
        "asr" => 4,
        "adc" => 5,
        "sbc" => 6,
        "ror" => 7,
        "tst" => 8,
        "neg" => 9,
        "cmp" => 10,
        "cmn" => 11,
        "orr" => 12,
        "mul" => 13,
        "bic" => 14,
        "mvn" => 15,
        _ => return None,
    };
    Some(op)
}

/// Returns the opcode of a load or store with a register offset.
fn register_offset(mnemonic: &str) -> Option<u16> {
    let opcode = match mnemonic {
        "str" => 0x5000,
        "strh" => 0x5200,
        "strb" => 0x5400,
        "ldrsb" | "ldsb" => 0x5600,
        "ldr" => 0x5800,
        "ldrh" => 0x5A00,
        "ldrb" => 0x5C00,
        "ldrsh" | "ldsh" => 0x5E00,
        _ => return None,
    };
    Some(opcode)
}

/// Returns the opcode of a load or store with an immediate offset, and the
/// size it accesses.
fn immediate_offset(mnemonic: &str) -> Option<(u16, i64)> {
    let opcode = match mnemonic {
        "str" => (0x6000, 4),
        "ldr" => (0x6800, 4),
        "strb" => (0x7000, 1),
        "ldrb" => (0x7800, 1),
        "strh" => (0x8000, 2),
        "ldrh" => (0x8800, 2),
        _ => return None,
    };
    Some(opcode)
}

/// Checks that a register list holds at least one register and only those in
/// `allowed`.
const fn check_list(list: u16, allowed: u16, names: &'static str) -> Result<u16, ErrorKind> {
    if list != 0 && list & !allowed == 0 {
        Ok(list)
    } else {
        Err(ErrorKind::List(names))
    }
}

/// The result of a group of encodings: `None` if the instruction isn't in the
/// group.
type Encoded = Result<Option<Vec<u16>>, ErrorKind>;

/// Returns an encoding of one halfword.
#[allow(clippy::unnecessary_wraps)]
fn one(code: u16) -> Encoded {
    Ok(Some(vec![code]))
}

/// Encodes an instruction as one halfword, or two for `bl`.
///
/// # Errors
///
/// Returns an [`ErrorKind`] if the mnemonic is unknown, it doesn't take the
/// operands, or a value or target is out of range.
pub fn encode(
    mnemonic: &str,
    operands: &[Operand],
    ctx: &Context<'_>,
) -> Result<Vec<u16>, ErrorKind> {
    for group in [data, transfer, control] {
        if let Some(code) = group(mnemonic, operands, ctx)? {
            return Ok(code);
        }
    }
    if MNEMONICS.contains(&mnemonic) || condition(mnemonic).is_some() || alu(mnemonic).is_some() {
        Err(ErrorKind::Operands(mnemonic.to_string()))
    } else {
        Err(ErrorKind::Unknown(mnemonic.to_string()))
    }
}

/// Encodes a data processing instruction or `bx`.
fn data(mnemonic: &str, operands: &[Operand], ctx: &Context<'_>) -> Encoded {
    use Operand::{Imm, Reg};

    let alu = alu(mnemonic);
    let is_sub = u16::from(mnemonic == "sub");
    match (mnemonic, operands) {
        ("nop", []) => one(0x46C0),
        ("lsl" | "lsr" | "asr", [Reg(d), Reg(s), Imm(shift)]) if low(*d) && low(*s) => {
            let (op, max) = match mnemonic {
                "lsl" => (0, 31),
                "lsr" => (1, 32),
                _ => (2, 32),
            };
            let shift = ctx.unsigned(shift, "the shift", max, 1)? & 0x1F;
            one(op << 11 | shift << 6 | r(*s) << 3 | r(*d))
        }
        ("add" | "sub", [Reg(d), Reg(s), Reg(n)]) if low(*d) && low(*s) && low(*n) => {
            one(0x1800 | is_sub << 9 | r(*n) << 6 | r(*s) << 3 | r(*d))
        }
        ("add" | "sub", [Reg(d), Reg(n)]) if low(*d) && low(*n) => {
            one(0x1800 | is_sub << 9 | r(*n) << 6 | r(*d) << 3 | r(*d))
        }
        ("add" | "sub", [Reg(13), Imm(value)]) => {
            let value = ctx.value(value)?;
            let value = if is_sub == 1 { -value } else { value };
            check_range(value, "the immediate", -508, 508)?;
            check_multiple(value, "the immediate", 4)?;
            one(0xB000 | u16::from(value < 0) << 7 | (value.abs() / 4) as u16)
        }
        ("add", [Reg(d), Reg(base @ (13 | 15)), Imm(value)]) if low(*d) => {
            let value = ctx.unsigned(value, "the immediate", MAX_WORD_OFFSET, 4)?;
            one(0xA000 | u16::from(*base == 13) << 11 | r(*d) << 8 | value)
        }
        ("add" | "sub", [Reg(d), Reg(s), Imm(value)]) if low(*d) && low(*s) => {
            let number = ctx.value(value)?;
            if d == s && !(0..=7).contains(&number) {
                let value = ctx.unsigned(value, "the immediate", 255, 1)?;
                one(0x3000 | is_sub << 11 | r(*d) << 8 | value)
            } else {
                let value = ctx.unsigned(value, "the immediate", 7, 1)?;
                one(0x1C00 | is_sub << 9 | value << 6 | r(*s) << 3 | r(*d))
            }
        }
        ("mov" | "cmp" | "add" | "sub", [Reg(d), Imm(value)]) if low(*d) => {
            let op = match mnemonic {
                "mov" => 0,
                "cmp" => 1,
                "add" => 2,
                _ => 3,
            };
            let value = ctx.unsigned(value, "the immediate", 255, 1)?;
            one(0x2000 | op << 11 | r(*d) << 8 | value)
        }
        ("mov", [Reg(d), Reg(s)]) if low(*d) && low(*s) => one(0x1C00 | r(*s) << 3 | r(*d)),
        ("mov" | "cmp" | "add", [Reg(d), Reg(s)]) if !low(*d) || !low(*s) => {
            let op = match mnemonic {
                "add" => 0,
                "cmp" => 1,
                _ => 2,
            };
            one(0x4400 | op << 8 | r(*d >> 3) << 7 | r(*s) << 3 | r(*d & 7))
        }
        (_, [Reg(d), Reg(s)]) if alu.is_some() && low(*d) && low(*s) => {
            one(0x4000 | alu.unwrap_or_default() << 6 | r(*s) << 3 | r(*d))
        }
        ("bx", [Reg(s)]) => one(0x4700 | r(*s) << 3),
        _ => Ok(None),
    }
}

/// Encodes a load or a store.
fn transfer(mnemonic: &str, operands: &[Operand], ctx: &Context<'_>) -> Encoded {
    use Operand::{Address, List, Literal, Mem, Reg, WriteBack};

    let register_offset = register_offset(mnemonic);
    let immediate_offset = immediate_offset(mnemonic);
    match (mnemonic, operands) {
        ("ldr", [Reg(d), Literal(value)]) if low(*d) => {
            let value = word(ctx.value(value)?, "the value")?;
            ctx.load(*d, ctx.literals.get(&value).copied().unwrap_or_default())
                .map(Some)
        }
        ("ldr", [Reg(d), Address(target)]) if low(*d) => ctx.load(*d, ctx.value(target)?).map(Some),
        (
            "ldr",
            [
                Reg(d),
                Mem {
                    base: 15,
                    offset: Offset::Imm(value),
                },
            ],
        ) if low(*d) => {
            let value = ctx.unsigned(value, "the offset", MAX_WORD_OFFSET, 4)?;
            one(0x4800 | r(*d) << 8 | value)
        }
        (
            "ldr" | "str",
            [
                Reg(d),
                Mem {
                    base: 13,
                    offset: Offset::Imm(value),
                },
            ],
        ) if low(*d) => {
            let load = u16::from(mnemonic == "ldr");
            let value = ctx.unsigned(value, "the offset", MAX_WORD_OFFSET, 4)?;
            one(0x9000 | load << 11 | r(*d) << 8 | value)
        }
        (
            _,
            [
                Reg(d),
                Mem {
                    base,
                    offset: Offset::Reg(o),
                },
            ],
        ) if register_offset.is_some() && low(*d) && low(*base) && low(*o) => {
            one(register_offset.unwrap_or_default() | r(*o) << 6 | r(*base) << 3 | r(*d))
        }
        (
            _,
            [
                Reg(d),
                Mem {
                    base,
                    offset: Offset::Imm(value),
                },
            ],
        ) if immediate_offset.is_some() && low(*d) && low(*base) => {
            let (opcode, size) = immediate_offset.unwrap_or_default();
            let value = ctx.unsigned(value, "the offset", 31 * size, size)?;
            one(opcode | value << 6 | r(*base) << 3 | r(*d))
        }
        ("push", [List(list)]) | ("stmdb", [WriteBack(13), List(list)]) => {
            let list = check_list(*list, 0x40FF, "r0-r7 and lr")?;
            one(0xB400 | u16::from(list & 0x4000 != 0) << 8 | (list & 0xFF))
        }
        ("pop", [List(list)]) | ("ldmia", [WriteBack(13), List(list)]) => {
            let list = check_list(*list, 0x80FF, "r0-r7 and pc")?;
            one(0xBC00 | u16::from(list & 0x8000 != 0) << 8 | (list & 0xFF))
        }
        ("stmia" | "ldmia", [WriteBack(base), List(list)]) if low(*base) => {
            let load = u16::from(mnemonic == "ldmia");
            let list = check_list(*list, 0xFF, "r0-r7")?;
            one(0xC000 | load << 11 | r(*base) << 8 | list)
        }
        _ => Ok(None),
    }
}

/// Encodes a branch or `swi`.
fn control(mnemonic: &str, operands: &[Operand], ctx: &Context<'_>) -> Encoded {
    use Operand::{Address, Imm};

    let condition = condition(mnemonic);
    match (mnemonic, operands) {
        ("b", [Address(target)]) => {
            let offset = ctx.branch(target, -2048, 2046)?;
            one(0xE000 | (offset >> 1) as u16 & 0x7FF)
        }
        (_, [Address(target)]) if condition.is_some() => {
            let offset = ctx.branch(target, -256, 254)?;
            one(0xD000 | condition.unwrap_or_default() << 8 | (offset >> 1) as u16 & 0xFF)
        }
        ("bl", [Address(target)]) => {
            let offset = ctx.branch(target, -(1 << 22), (1 << 22) - 2)?;
            Ok(Some(vec![
                0xF000 | (offset >> 12) as u16 & 0x7FF,
                0xF800 | (offset >> 1) as u16 & 0x7FF,
            ]))
        }
        ("swi", [Imm(value)]) => one(0xDF00 | ctx.unsigned(value, "the comment", 255, 1)?),
        _ => Ok(None),
    }
}
