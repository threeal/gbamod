//! Assembling Thumb code, such as hooks loaded into free space in a ROM.

mod encode;
mod parse;

use crate::disasm;
use encode::Context;
use parse::{Expr, Operand, Statement};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The reason a line couldn't be assembled.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ErrorKind {
    /// The mnemonic or directive isn't known.
    #[error("`{0}` isn't an instruction or directive")]
    Unknown(String),
    /// The instruction doesn't take the operands it's given.
    #[error("`{0}` doesn't take these operands")]
    Operands(String),
    /// An operand can't be parsed.
    #[error("`{0}` isn't an operand")]
    InvalidOperand(String),
    /// A value is neither a number nor a name.
    #[error("`{0}` isn't a value; expected numbers and names joined by + or -")]
    InvalidValue(String),
    /// A label isn't a valid name.
    #[error("`{0}` isn't a label; expected a letter or _ followed by letters, digits, or _")]
    InvalidLabel(String),
    /// `.align` isn't given a power of 2 it takes.
    #[error("`.align` takes a power of 2 from 0 to {max}, not `{0}`", max = parse::MAX_ALIGN)]
    InvalidAlign(String),
    /// `.equ` isn't given a name and a value.
    #[error("`.equ` takes a name and a value, not `{0}`")]
    InvalidEqu(String),
    /// A name isn't defined.
    #[error("`{0}` isn't defined")]
    Undefined(String),
    /// A name is defined twice.
    #[error("`{0}` is already defined")]
    Duplicate(String),
    /// An instruction follows data that ends at an odd address.
    #[error("an instruction can't start at the odd address 0x{0:08X}")]
    OddAddress(i64),
    /// A value is out of range.
    #[error("{what} {value} is out of range; expected {min} to {max}")]
    Range {
        /// What the value is for.
        what: &'static str,
        /// The value.
        value: i64,
        /// The smallest value allowed.
        min: i64,
        /// The largest value allowed.
        max: i64,
    },
    /// A value isn't a multiple of the size it's scaled by.
    #[error("{what} {value} isn't a multiple of {multiple}")]
    Unaligned {
        /// What the value is for.
        what: &'static str,
        /// The value.
        value: i64,
        /// The multiple it must be.
        multiple: i64,
    },
    /// A branch targets an odd address.
    #[error("the target 0x{0:08X} is odd, but Thumb code is at even addresses")]
    OddTarget(u32),
    /// A branch or load can't reach its target.
    #[error("the target 0x{target:08X} is {offset} bytes away; expected {min} to {max}")]
    Reach {
        /// The target.
        target: u32,
        /// Its distance from the instruction's `pc`.
        offset: i64,
        /// The smallest distance reached.
        min: i64,
        /// The largest distance reached.
        max: i64,
    },
    /// A register list is empty or holds a register the instruction can't
    /// transfer.
    #[error("the register list must hold at least one of {0}, and nothing else")]
    List(&'static str),
}

/// The reason code couldn't be assembled.
#[derive(Debug, thiserror::Error)]
pub enum AsmError {
    /// The code is placed at an odd address.
    #[error("Thumb code starts at an even address, but 0x{0:08X} isn't one")]
    OddOrigin(u32),
    /// A line couldn't be assembled.
    #[error("line {line}: {kind}")]
    Line {
        /// The line's number, from 1.
        line: usize,
        /// Why it couldn't be assembled.
        kind: ErrorKind,
    },
    /// The code doesn't fit below the end of the address space.
    #[error("the code runs past the end of the address space")]
    Overflow,
    /// The source file couldn't be read.
    #[error("failed to read `{}`: {source}", path.display())]
    Read {
        /// The source file.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// A line of the source file couldn't be assembled.
    #[error("`{}` line {line}: {kind}", path.display())]
    File {
        /// The source file.
        path: PathBuf,
        /// The line's number, from 1.
        line: usize,
        /// Why it couldn't be assembled.
        kind: ErrorKind,
    },
}

/// Assembled code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    /// The address the code starts at.
    pub origin: u32,
    /// The code, followed by its literal pool.
    pub bytes: Vec<u8>,
    /// A line for each label, instruction, and data value, with its address
    /// and encoding.
    pub listing: Vec<String>,
}

/// A label, directive, or instruction placed at an address.
struct Item {
    line: usize,
    address: i64,
    text: String,
    kind: Kind,
}

enum Kind {
    Label(String),
    Data {
        size: u8,
        values: Vec<Expr>,
    },
    Padding(i64),
    Instruction {
        mnemonic: String,
        operands: Vec<Operand>,
    },
}

/// The items of the code, placed at their addresses.
struct Layout {
    items: Vec<Item>,
    /// The address of each label and the value of each constant.
    symbols: HashMap<String, i64>,
    /// The address the code ends at, before its literal pool.
    end: i64,
}

/// The first address past the end of the address space.
const END: i64 = 1 << 32;

impl Program {
    /// Returns the listing, followed by how many bytes were assembled.
    #[must_use]
    pub fn report(&self) -> String {
        let count = self.bytes.len();
        let plural = if count == 1 { "" } else { "s" };
        let mut lines = self.listing.clone();
        lines.push(format!(
            "Assembled {count} byte{plural} at 0x{:08X}",
            self.origin
        ));
        lines.join("\n")
    }
}

/// Assembles Thumb code placed at `origin`.
///
/// # Errors
///
/// Returns an [`AsmError`] if `origin` is odd, a line can't be assembled, or
/// the code runs past the end of the address space.
pub fn assemble(source: &str, origin: u32) -> Result<Program, AsmError> {
    assemble_with(source, origin, &[])
}

/// Assembles Thumb code placed at `origin`, with `names` defined before its
/// first line as if by `.equ`.
///
/// # Errors
///
/// Returns an [`AsmError`] if `origin` is odd, a line can't be assembled,
/// including one that defines a name again, or the code runs past the end of
/// the address space.
pub fn assemble_with(
    source: &str,
    origin: u32,
    names: &[(&str, u32)],
) -> Result<Program, AsmError> {
    if !origin.is_multiple_of(2) {
        return Err(AsmError::OddOrigin(origin));
    }
    let symbols = names
        .iter()
        .map(|&(name, value)| (name.to_string(), i64::from(value)))
        .collect();
    let Layout {
        items,
        symbols,
        end,
    } = layout(source, origin, symbols)?;
    let (pool, pool_start) = pool(&items, &symbols, end)?;
    let mut literals = HashMap::new();
    let mut address = pool_start;
    for &value in &pool {
        literals.insert(value, address);
        address += 4;
    }
    if address > END {
        return Err(AsmError::Overflow);
    }

    let mut bytes = Vec::new();
    let mut listing = Vec::new();
    for item in &items {
        let error = |kind| AsmError::Line {
            line: item.line,
            kind,
        };
        match &item.kind {
            Kind::Label(name) => listing.push(format!("{:08X} <{name}>:", to_u32(item.address))),
            Kind::Data { size, values } => {
                let mut address = item.address;
                for (index, value) in values.iter().enumerate() {
                    let value = data(value, *size, &symbols).map_err(error)?;
                    let width = usize::from(*size);
                    bytes.extend_from_slice(&value.to_le_bytes()[..width]);
                    let text = if index == 0 { item.text.as_str() } else { "" };
                    let encoding = format!("{value:0digits$X}", digits = width * 2);
                    listing.push(disasm::row(to_u32(address), &encoding, text));
                    address += i64::from(*size);
                }
            }
            Kind::Padding(0) => {}
            Kind::Padding(size) => {
                bytes.extend((0..*size).map(|_| 0));
                let encoding: String = (0..*size).map(|_| "00").collect();
                listing.push(disasm::row(to_u32(item.address), &encoding, &item.text));
            }
            Kind::Instruction { mnemonic, operands } => {
                let ctx = Context {
                    address: item.address,
                    symbols: &symbols,
                    literals: &literals,
                };
                let halves = encode::encode(mnemonic, operands, &ctx).map_err(error)?;
                for half in &halves {
                    bytes.extend_from_slice(&half.to_le_bytes());
                }
                let encoding: Vec<String> =
                    halves.iter().map(|half| format!("{half:04X}")).collect();
                listing.push(disasm::row(
                    to_u32(item.address),
                    &encoding.join(" "),
                    &item.text,
                ));
            }
        }
    }
    if pool_start > end {
        let size = pool_start - end;
        bytes.extend((0..size).map(|_| 0));
        let encoding: String = (0..size).map(|_| "00").collect();
        listing.push(disasm::row(to_u32(end), &encoding, ""));
    }
    for value in pool {
        bytes.extend_from_slice(&value.to_le_bytes());
        listing.push(disasm::row(
            to_u32(literals[&value]),
            &format!("{value:08X}"),
            &format!(".word 0x{value:08X}"),
        ));
    }
    Ok(Program {
        origin,
        bytes,
        listing,
    })
}

/// Assembles the Thumb code in the file at `path` placed at `origin`, and
/// lists it, for the `asm` command.
///
/// # Errors
///
/// Returns an [`AsmError`] if the file can't be read or its code can't be
/// assembled.
pub fn run(path: &Path, origin: u32) -> Result<String, AsmError> {
    let source = std::fs::read_to_string(path).map_err(|source| AsmError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let program = assemble(&source, origin).map_err(|err| match err {
        AsmError::Line { line, kind } => AsmError::File {
            path: path.to_path_buf(),
            line,
            kind,
        },
        err => err,
    })?;
    Ok(program.report())
}

/// Places each label, directive, and instruction at its address, and
/// defines each label and constant.
fn layout(
    source: &str,
    origin: u32,
    mut symbols: HashMap<String, i64>,
) -> Result<Layout, AsmError> {
    let mut items = Vec::new();
    let mut address = i64::from(origin);
    for (index, text) in source.lines().enumerate() {
        let line = index + 1;
        let error = |kind| AsmError::Line { line, kind };
        let parsed = parse::line(text).map_err(error)?;
        if let Some(name) = parsed.label {
            define(&mut symbols, &name, address).map_err(error)?;
            items.push(Item {
                line,
                address,
                text: String::new(),
                kind: Kind::Label(name),
            });
        }
        let (kind, size) = match parsed.statement {
            None => continue,
            Some(Statement::Equ(name, value)) => {
                let value = value.eval(&symbols).map_err(error)?;
                define(&mut symbols, &name, value).map_err(error)?;
                continue;
            }
            Some(Statement::Data { size, values }) => {
                let count = i64::try_from(values.len()).unwrap_or(i64::MAX);
                (Kind::Data { size, values }, i64::from(size) * count)
            }
            Some(Statement::Align(power)) => {
                let size = padding(address, 1 << power);
                (Kind::Padding(size), size)
            }
            Some(Statement::Instruction { mnemonic, operands }) => {
                if address % 2 != 0 {
                    return Err(error(ErrorKind::OddAddress(address)));
                }
                let size = if mnemonic == "bl" { 4 } else { 2 };
                (Kind::Instruction { mnemonic, operands }, size)
            }
        };
        items.push(Item {
            line,
            address,
            text: parsed.text,
            kind,
        });
        address += size;
    }
    Ok(Layout {
        items,
        symbols,
        end: address,
    })
}

/// Returns each distinct value loaded with `=`, in order of first use, and
/// the word-aligned address their pool starts at, after the code.
fn pool(
    items: &[Item],
    symbols: &HashMap<String, i64>,
    end: i64,
) -> Result<(Vec<u32>, i64), AsmError> {
    let mut pool = Vec::new();
    for item in items {
        let Kind::Instruction { operands, .. } = &item.kind else {
            continue;
        };
        for operand in operands {
            let Operand::Literal(value) = operand else {
                continue;
            };
            let value = value
                .eval(symbols)
                .and_then(|value| encode::word(value, "the value"))
                .map_err(|kind| AsmError::Line {
                    line: item.line,
                    kind,
                })?;
            if !pool.contains(&value) {
                pool.push(value);
            }
        }
    }
    let start = if pool.is_empty() {
        end
    } else {
        end + padding(end, 4)
    };
    Ok((pool, start))
}

fn define(symbols: &mut HashMap<String, i64>, name: &str, value: i64) -> Result<(), ErrorKind> {
    if symbols.contains_key(name) {
        return Err(ErrorKind::Duplicate(name.to_string()));
    }
    symbols.insert(name.to_string(), value);
    Ok(())
}

/// Returns the bytes needed to pad `address` to a multiple of `align`.
const fn padding(address: i64, align: i64) -> i64 {
    (align - address % align) % align
}

/// Evaluates a data value of `size` bytes, as either a signed or unsigned
/// number.
fn data(value: &Expr, size: u8, symbols: &HashMap<String, i64>) -> Result<u32, ErrorKind> {
    let bits = 8 * u32::from(size);
    let value = value.eval(symbols)?;
    encode::check_range(value, "the value", -(1 << (bits - 1)), (1 << bits) - 1)?;
    Ok(encode::word(value, "the value")? & (u32::MAX >> (32 - bits)))
}

/// Narrows an address already checked to be below [`END`].
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
const fn to_u32(address: i64) -> u32 {
    address as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// One of each instruction form, checked against mGBA's disassembler.
    const FORMS: &str = "\
        start:\n\
        lsl r1, r2, #5\n\
        lsr r1, r2, #32\n\
        asr r7, r0, #1\n\
        add r1, r2, r3\n\
        sub r1, r2, r3\n\
        add r1, r2\n\
        sub r4, r5\n\
        add r1, r2, #7\n\
        sub r1, r2, #3\n\
        add r1, r1, #200\n\
        mov r3, #255\n\
        cmp r3, #1\n\
        add r3, #4\n\
        sub r3, #4\n\
        mov r0, r1\n\
        and r0, r1\n\
        eor r0, r1\n\
        lsl r0, r1\n\
        lsr r0, r1\n\
        asr r0, r1\n\
        adc r0, r1\n\
        sbc r0, r1\n\
        ror r0, r1\n\
        tst r0, r1\n\
        neg r0, r1\n\
        cmp r0, r1\n\
        cmn r0, r1\n\
        orr r0, r1\n\
        mul r0, r1\n\
        bic r0, r1\n\
        mvn r0, r1\n\
        add r1, r9\n\
        add r8, r2\n\
        cmp r8, r0\n\
        mov r8, r8\n\
        mov r0, sp\n\
        nop\n\
        bx r1\n\
        bx lr\n\
        ldr r0, [pc, #8]\n\
        str r0, [r1, r2]\n\
        strh r0, [r1, r2]\n\
        strb r0, [r1, r2]\n\
        ldrsb r0, [r1, r2]\n\
        ldr r0, [r1, r2]\n\
        ldrh r0, [r1, r2]\n\
        ldrb r0, [r1, r2]\n\
        ldrsh r0, [r1, r2]\n\
        str r0, [r1, #124]\n\
        ldr r0, [r1, #4]\n\
        strb r0, [r1, #31]\n\
        ldrb r0, [r1]\n\
        strh r0, [r1, #62]\n\
        ldrh r0, [r1, #2]\n\
        str r0, [sp, #1020]\n\
        ldr r0, [sp, #4]\n\
        add r0, pc, #4\n\
        add r0, sp, #1020\n\
        add sp, #16\n\
        sub sp, #16\n\
        add sp, #-8\n\
        push {r4-r6, lr}\n\
        pop {r4, pc}\n\
        stmdb sp!, {r0}\n\
        ldmia sp!, {r1}\n\
        stmia r0!, {r2-r3, r7}\n\
        ldmia r1!, {r0}\n\
        beq start\n\
        bne start\n\
        bcs start\n\
        bcc start\n\
        bmi start\n\
        bpl start\n\
        bvs start\n\
        bvc start\n\
        bhi start\n\
        bls start\n\
        bge start\n\
        blt start\n\
        bgt start\n\
        ble start\n\
        swi #240\n\
        b start\n\
        bl 0x02031000\n\
        bl start\n\
        ldr r5, =0x12345678";

    const FORMS_LISTING: &str = "\
        02030000 <start>:\n\
        02030000: 0151       lsl r1, r2, #5\n\
        02030002: 0811       lsr r1, r2, #32\n\
        02030004: 1047       asr r7, r0, #1\n\
        02030006: 18D1       add r1, r2, r3\n\
        02030008: 1AD1       sub r1, r2, r3\n\
        0203000A: 1889       add r1, r2\n\
        0203000C: 1B64       sub r4, r5\n\
        0203000E: 1DD1       add r1, r2, #7\n\
        02030010: 1ED1       sub r1, r2, #3\n\
        02030012: 31C8       add r1, r1, #200\n\
        02030014: 23FF       mov r3, #255\n\
        02030016: 2B01       cmp r3, #1\n\
        02030018: 3304       add r3, #4\n\
        0203001A: 3B04       sub r3, #4\n\
        0203001C: 1C08       mov r0, r1\n\
        0203001E: 4008       and r0, r1\n\
        02030020: 4048       eor r0, r1\n\
        02030022: 4088       lsl r0, r1\n\
        02030024: 40C8       lsr r0, r1\n\
        02030026: 4108       asr r0, r1\n\
        02030028: 4148       adc r0, r1\n\
        0203002A: 4188       sbc r0, r1\n\
        0203002C: 41C8       ror r0, r1\n\
        0203002E: 4208       tst r0, r1\n\
        02030030: 4248       neg r0, r1\n\
        02030032: 4288       cmp r0, r1\n\
        02030034: 42C8       cmn r0, r1\n\
        02030036: 4308       orr r0, r1\n\
        02030038: 4348       mul r0, r1\n\
        0203003A: 4388       bic r0, r1\n\
        0203003C: 43C8       mvn r0, r1\n\
        0203003E: 4449       add r1, r9\n\
        02030040: 4490       add r8, r2\n\
        02030042: 4580       cmp r8, r0\n\
        02030044: 46C0       mov r8, r8\n\
        02030046: 4668       mov r0, sp\n\
        02030048: 46C0       nop\n\
        0203004A: 4708       bx r1\n\
        0203004C: 4770       bx lr\n\
        0203004E: 4802       ldr r0, [pc, #8]\n\
        02030050: 5088       str r0, [r1, r2]\n\
        02030052: 5288       strh r0, [r1, r2]\n\
        02030054: 5488       strb r0, [r1, r2]\n\
        02030056: 5688       ldrsb r0, [r1, r2]\n\
        02030058: 5888       ldr r0, [r1, r2]\n\
        0203005A: 5A88       ldrh r0, [r1, r2]\n\
        0203005C: 5C88       ldrb r0, [r1, r2]\n\
        0203005E: 5E88       ldrsh r0, [r1, r2]\n\
        02030060: 67C8       str r0, [r1, #124]\n\
        02030062: 6848       ldr r0, [r1, #4]\n\
        02030064: 77C8       strb r0, [r1, #31]\n\
        02030066: 7808       ldrb r0, [r1]\n\
        02030068: 87C8       strh r0, [r1, #62]\n\
        0203006A: 8848       ldrh r0, [r1, #2]\n\
        0203006C: 90FF       str r0, [sp, #1020]\n\
        0203006E: 9801       ldr r0, [sp, #4]\n\
        02030070: A001       add r0, pc, #4\n\
        02030072: A8FF       add r0, sp, #1020\n\
        02030074: B004       add sp, #16\n\
        02030076: B084       sub sp, #16\n\
        02030078: B082       add sp, #-8\n\
        0203007A: B570       push {r4-r6, lr}\n\
        0203007C: BD10       pop {r4, pc}\n\
        0203007E: B401       stmdb sp!, {r0}\n\
        02030080: BC02       ldmia sp!, {r1}\n\
        02030082: C08C       stmia r0!, {r2-r3, r7}\n\
        02030084: C901       ldmia r1!, {r0}\n\
        02030086: D0BB       beq start\n\
        02030088: D1BA       bne start\n\
        0203008A: D2B9       bcs start\n\
        0203008C: D3B8       bcc start\n\
        0203008E: D4B7       bmi start\n\
        02030090: D5B6       bpl start\n\
        02030092: D6B5       bvs start\n\
        02030094: D7B4       bvc start\n\
        02030096: D8B3       bhi start\n\
        02030098: D9B2       bls start\n\
        0203009A: DAB1       bge start\n\
        0203009C: DBB0       blt start\n\
        0203009E: DCAF       bgt start\n\
        020300A0: DDAE       ble start\n\
        020300A2: DFF0       swi #240\n\
        020300A4: E7AC       b start\n\
        020300A6: F000 FFAB  bl 0x02031000\n\
        020300AA: F7FF FFA9  bl start\n\
        020300AE: 4D00       ldr r5, =0x12345678\n\
        020300B0: 12345678   .word 0x12345678";

    /// A hook that calls a function, then changes its result if the first
    /// argument matches a value in memory.
    const HOOK: &str = "\
        @ Calls the original, then clears its result for the player.
        .equ ORIGINAL, 0x08001235   @ Thumb
        .equ PLAYER, 0x02000100

                push {r4, lr}
                ldr r4, =ORIGINAL
                bl call_r4
                ldr r1, =PLAYER
                ldr r1, [r1]
                ldr r2, =ORIGINAL   @ reuses the first literal
                cmp r1, #0
                beq done
                mov r0, #0
        done:   pop {r4}
                pop {r1}
                bx r1
        call_r4:
                bx r4
    ";

    /// Returns the error of assembling `source` at 0x08000000.
    fn error(source: &str) -> String {
        assemble(source, 0x0800_0000).unwrap_err().to_string()
    }

    #[test]
    fn encode_every_form() {
        let program = assemble(FORMS, 0x0203_0000).unwrap();
        assert_eq!(program.listing.join("\n"), FORMS_LISTING);
        assert_eq!(program.bytes.len(), 180);
        assert_eq!(program.bytes[..4], [0x51, 0x01, 0x11, 0x08]);
        assert_eq!(program.bytes[176..], [0x78, 0x56, 0x34, 0x12]);
    }

    #[test]
    fn assemble_hook() {
        let program = assemble(HOOK, 0x08CB_F000).unwrap();
        let halves: Vec<u16> = program.bytes[..28]
            .chunks(2)
            .map(|half| u16::from_le_bytes([half[0], half[1]]))
            .collect();
        assert_eq!(
            halves,
            [
                0xB510, 0x4C06, 0xF000, 0xF809, 0x4905, 0x6809, 0x4A03, 0x2900, 0xD000, 0x2000,
                0xBC10, 0xBC02, 0x4708, 0x4720
            ]
        );
        assert_eq!(
            program.bytes[28..],
            [0x35, 0x12, 0x00, 0x08, 0x00, 0x01, 0x00, 0x02]
        );
        assert_eq!(
            program.report(),
            "\
08CBF000: B510       push {r4, lr}
08CBF002: 4C06       ldr r4, =ORIGINAL
08CBF004: F000 F809  bl call_r4
08CBF008: 4905       ldr r1, =PLAYER
08CBF00A: 6809       ldr r1, [r1]
08CBF00C: 4A03       ldr r2, =ORIGINAL
08CBF00E: 2900       cmp r1, #0
08CBF010: D000       beq done
08CBF012: 2000       mov r0, #0
08CBF014 <done>:
08CBF014: BC10       pop {r4}
08CBF016: BC02       pop {r1}
08CBF018: 4708       bx r1
08CBF01A <call_r4>:
08CBF01A: 4720       bx r4
08CBF01C: 08001235   .word 0x08001235
08CBF020: 02000100   .word 0x02000100
Assembled 36 bytes at 0x08CBF000"
        );
    }

    #[test]
    fn pad_literal_pool() {
        let program = assemble("ldr r0, =0x1234\nbx r0", 0x0800_0000).unwrap();
        assert_eq!(
            program.listing,
            [
                "08000000: 4800       ldr r0, =0x1234",
                "08000002: 4700       bx r0",
                "08000004: 00001234   .word 0x00001234",
            ]
        );
        let program = assemble("ldr r0, =-1", 0x0800_0000).unwrap();
        assert_eq!(program.bytes, [0x00, 0x48, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(program.listing[1], "08000002: 0000");
    }

    #[test]
    fn place_data() {
        let program = assemble(
            "table: .byte 1, -1\n.align 2\n.hword 0xBEEF\n.word table+1\n.align 2\nnop\n.align 1\n.word",
            0x0800_0000,
        )
        .unwrap();
        assert_eq!(
            program.listing,
            [
                "08000000 <table>:",
                "08000000: 01         .byte 1, -1",
                "08000001: FF",
                "08000002: 0000       .align 2",
                "08000004: BEEF       .hword 0xBEEF",
                "08000006: 08000001   .word table+1",
                "0800000A: 0000       .align 2",
                "0800000C: 46C0       nop",
            ]
        );
        assert_eq!(
            program.bytes,
            [1, 0xFF, 0, 0, 0xEF, 0xBE, 1, 0, 0, 8, 0, 0, 0xC0, 0x46]
        );
        assert_eq!(
            assemble(".byte 1", 0).unwrap().report(),
            "00000000: 01         .byte 1\nAssembled 1 byte at 0x00000000"
        );
    }

    #[test]
    fn reject_bad_data() {
        assert_eq!(
            error(".byte 256"),
            "line 1: the value 256 is out of range; expected -128 to 255"
        );
        assert_eq!(
            error(".hword -32769"),
            "line 1: the value -32769 is out of range; expected -32768 to 65535"
        );
        assert_eq!(
            error(".word 0xFFFFFFFF+1"),
            "line 1: the value 4294967296 is out of range; expected -2147483648 to 4294967295"
        );
        assert_eq!(
            error(".byte 1\nnop"),
            "line 2: an instruction can't start at the odd address 0x08000001"
        );
        assert_eq!(error(".word MISSING"), "line 1: `MISSING` isn't defined");
    }

    #[test]
    fn define_names() {
        assert_eq!(error("a: nop\na: nop"), "line 2: `a` is already defined");
        assert_eq!(
            error(".equ A, 1\n\n.equ A, 2"),
            "line 3: `A` is already defined"
        );
        assert_eq!(error(".equ A, B\n.equ B, 1"), "line 1: `B` isn't defined");
        assert_eq!(
            assemble("start: b start + 2 - 2", 0x0800_0000)
                .unwrap()
                .bytes,
            [0xFE, 0xE7]
        );
        assert_eq!(error("ldr r0, =MISSING"), "line 1: `MISSING` isn't defined");
        assert_eq!(
            error("ldr r0, =0xFFFFFFFF+1"),
            "line 1: the value 4294967296 is out of range; expected -2147483648 to 4294967295"
        );
    }

    #[test]
    fn reject_bad_lines() {
        assert_eq!(
            error("nop\n1x: nop"),
            "line 2: `1x` isn't a label; expected a letter or _ followed by letters, digits, or _"
        );
        assert_eq!(
            error("foo r0"),
            "line 1: `foo` isn't an instruction or directive"
        );
        assert_eq!(
            error(".pool"),
            "line 1: `.pool` isn't an instruction or directive"
        );
        assert_eq!(
            error("mov r0, 1 1"),
            "line 1: `1 1` isn't a value; expected numbers and names joined by + or -"
        );
        assert_eq!(
            error("ldr r0, [r1, x2]"),
            "line 1: `[r1, x2]` isn't an operand"
        );
        assert_eq!(
            error(".align 5"),
            "line 1: `.align` takes a power of 2 from 0 to 4, not `5`"
        );
        assert_eq!(
            error(".equ A"),
            "line 1: `.equ` takes a name and a value, not `A`"
        );
    }

    #[test]
    fn reject_bad_operands() {
        for (source, message) in [
            ("mov r0", "`mov` doesn't take these operands"),
            ("beq r0", "`beq` doesn't take these operands"),
            ("and r8, r1", "`and` doesn't take these operands"),
            ("ldr r8, [r0]", "`ldr` doesn't take these operands"),
            (
                "lsl r0, r1, #32",
                "the shift 32 is out of range; expected 0 to 31",
            ),
            (
                "lsr r0, r1, #33",
                "the shift 33 is out of range; expected 0 to 32",
            ),
            (
                "add r0, r1, #8",
                "the immediate 8 is out of range; expected 0 to 7",
            ),
            (
                "add r0, r0, #256",
                "the immediate 256 is out of range; expected 0 to 255",
            ),
            (
                "mov r0, #-1",
                "the immediate -1 is out of range; expected 0 to 255",
            ),
            (
                "add sp, #512",
                "the immediate 512 is out of range; expected -508 to 508",
            ),
            ("sub sp, #6", "the immediate -6 isn't a multiple of 4"),
            ("add r0, sp, #3", "the immediate 3 isn't a multiple of 4"),
            ("ldr r0, [r1, #2]", "the offset 2 isn't a multiple of 4"),
            (
                "ldrb r0, [r1, #32]",
                "the offset 32 is out of range; expected 0 to 31",
            ),
            (
                "ldrh r0, [r1, #64]",
                "the offset 64 is out of range; expected 0 to 62",
            ),
            (
                "ldr r0, [pc, #1024]",
                "the offset 1024 is out of range; expected 0 to 1020",
            ),
            (
                "str r0, [sp, #-4]",
                "the offset -4 is out of range; expected 0 to 1020",
            ),
            (
                "swi #256",
                "the comment 256 is out of range; expected 0 to 255",
            ),
            (
                "push {r8}",
                "the register list must hold at least one of r0-r7 and lr, and nothing else",
            ),
            (
                "pop {}",
                "the register list must hold at least one of r0-r7 and pc, and nothing else",
            ),
            (
                "stmia r0!, {lr}",
                "the register list must hold at least one of r0-r7, and nothing else",
            ),
            (
                "b 0x08000001",
                "the target 0x08000001 is odd, but Thumb code is at even addresses",
            ),
            (
                "b -0x80000001",
                "the target -2147483649 is out of range; expected -2147483648 to 4294967295",
            ),
            (
                "b 0x08000804",
                "the target 0x08000804 is 2048 bytes away; expected -2048 to 2046",
            ),
            (
                "beq 0x07FFFF02",
                "the target 0x07FFFF02 is -258 bytes away; expected -256 to 254",
            ),
            (
                "bl 0x08400004",
                "the target 0x08400004 is 4194304 bytes away; expected -4194304 to 4194302",
            ),
            ("ldr r0, 0x08000006", "the offset 2 isn't a multiple of 4"),
            (
                "ldr r0, 0x08000404",
                "the target 0x08000404 is 1024 bytes away; expected 0 to 1020",
            ),
            (
                "ldr r0, 0x07FFFFFC",
                "the target 0x07FFFFFC is -8 bytes away; expected 0 to 1020",
            ),
            (
                "ldr r0, -0x80000001",
                "the target -2147483649 is out of range; expected -2147483648 to 4294967295",
            ),
        ] {
            assert_eq!(error(source), format!("line 1: {message}"), "{source}");
        }
    }

    #[test]
    fn encode_other_forms() {
        let code = |source| assemble(source, 0x0800_0000).unwrap().bytes;
        assert_eq!(code("sub sp, #-8"), [0x02, 0xB0]);
        assert_eq!(code("ldr r0, 0x08000008"), [0x01, 0x48]);
        assert_eq!(
            code("bcc 0x08000000\nblo 0x08000000\nbhs 0x08000000"),
            [0xFE, 0xD3, 0xFD, 0xD3, 0xFC, 0xD2]
        );
        assert_eq!(
            code("ldsb r0, [r1, r2]\nldsh r0, [r1, r2]"),
            [0x88, 0x56, 0x88, 0x5E]
        );
        assert_eq!(code("pop {r0}\nldmia r0!, {r1}"), [0x01, 0xBC, 0x02, 0xC8]);
    }

    #[test]
    fn reject_odd_origin() {
        assert_eq!(
            assemble("nop", 0x0800_0001).unwrap_err().to_string(),
            "Thumb code starts at an even address, but 0x08000001 isn't one"
        );
    }

    #[test]
    fn reject_code_past_address_space() {
        assert!(assemble(".word 0, 0", 0xFFFF_FFF8).is_ok());
        assert!(matches!(
            assemble(".word 0, 0, 0", 0xFFFF_FFF8),
            Err(AsmError::Overflow)
        ));
        assert_eq!(
            assemble("ldr r0, =1\nnop\nnop", 0xFFFF_FFF8)
                .unwrap_err()
                .to_string(),
            "the code runs past the end of the address space"
        );
    }

    #[test]
    fn define_names_before_code() {
        let program = assemble_with(
            "ldr r3, =ORIGINAL\nbx r3",
            0x0800_0000,
            &[("ORIGINAL", 0x0804_FD09)],
        )
        .unwrap();
        assert_eq!(
            program.bytes,
            [0x00, 0x4B, 0x18, 0x47, 0x09, 0xFD, 0x04, 0x08]
        );
        assert_eq!(
            assemble_with("ORIGINAL: nop", 0x0800_0000, &[("ORIGINAL", 1)])
                .unwrap_err()
                .to_string(),
            "line 1: `ORIGINAL` is already defined"
        );
    }

    #[test]
    fn run_on_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hook.s");
        fs::write(&path, "nop\n\nbx lr\n").unwrap();
        assert_eq!(
            run(&path, 0x0800_0000).unwrap(),
            "08000000: 46C0       nop\n08000002: 4770       bx lr\nAssembled 4 bytes at 0x08000000"
        );

        fs::write(&path, "nop\n\nbx r16\n").unwrap();
        assert_eq!(
            run(&path, 0x0800_0000).unwrap_err().to_string(),
            format!(
                "`{}` line 3: `bx` doesn't take these operands",
                path.display()
            )
        );
        assert!(matches!(
            run(&path, 0x0800_0001),
            Err(AsmError::OddOrigin(0x0800_0001))
        ));

        let missing = dir.path().join("missing.s");
        let err = run(&missing, 0x0800_0000).unwrap_err();
        assert!(matches!(err, AsmError::Read { .. }));
        assert!(
            err.to_string()
                .starts_with(&format!("failed to read `{}`: ", missing.display()))
        );
    }
}
