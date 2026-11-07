//! Watchpoints, breakpoints, and what the CPU held when they fired.

use std::fmt::{self, Write};
use std::str::FromStr;

/// How many words of the stack a hit records, and so the most a breakpoint
/// can show.
pub const MAX_STACK_WORDS: u32 = 64;

/// How many stack words a breakpoint hit prints per line.
const STACK_WORDS_PER_LINE: usize = 8;

/// The accesses that fire a watchpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WatchKind {
    /// Any write.
    Write,
    /// Any read.
    Read,
    /// Any read or write.
    Access,
    /// A write that changes the value.
    Change,
}

/// The reason text doesn't name a kind of watchpoint.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` isn't a kind of watchpoint; expected write, read, access, or change")]
pub struct WatchKindError(String);

impl FromStr for WatchKind {
    type Err = WatchKindError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "write" => Ok(Self::Write),
            "read" => Ok(Self::Read),
            "access" => Ok(Self::Access),
            "change" => Ok(Self::Change),
            _ => Err(WatchKindError(text.to_string())),
        }
    }
}

impl fmt::Display for WatchKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Write => "writes",
            Self::Read => "reads",
            Self::Access => "reads and writes",
            Self::Change => "changes",
        })
    }
}

/// What a point stops on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointKind {
    /// A watchpoint on memory.
    Watch(WatchKind),
    /// A breakpoint on an instruction, recording some words of the stack.
    Break {
        /// The number of stack words to show for each hit.
        stack: u32,
    },
}

/// A watchpoint or breakpoint set in a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    /// The id the emulator gave the point.
    pub id: i64,
    /// The address watched, or of the instruction to break on.
    pub address: u32,
    /// What the point stops on.
    pub kind: PointKind,
}

impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            PointKind::Watch(kind) => write!(f, "watching {kind} to 0x{:08X}", self.address),
            PointKind::Break { stack: 0 } => write!(f, "breaking at 0x{:08X}", self.address),
            PointKind::Break { stack } => write!(
                f,
                "breaking at 0x{:08X} with {stack} stack words",
                self.address
            ),
        }
    }
}

/// The CPU's registers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Registers {
    /// `r0` to `r15`, where `r15` holds the address of the instruction in
    /// question rather than the pipeline's lead.
    pub general: [u32; 16],
    /// The program status register.
    pub cpsr: u32,
}

impl Registers {
    /// Returns whether the CPU is running Thumb code.
    #[must_use]
    pub const fn thumb(&self) -> bool {
        self.cpsr & 0x20 != 0
    }

    /// Returns the stack pointer, `r13`.
    #[must_use]
    pub const fn sp(&self) -> u32 {
        self.general[13]
    }

    /// Returns the link register, `r14`.
    #[must_use]
    pub const fn lr(&self) -> u32 {
        self.general[14]
    }

    /// Returns the program counter, `r15`.
    #[must_use]
    pub const fn pc(&self) -> u32 {
        self.general[15]
    }
}

impl fmt::Display for Registers {
    /// Writes the registers four to a line, then `cpsr` and the CPU state.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const NAMES: [&str; 16] = [
            "r0", "r1", "r2", "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10", "r11", "r12", "sp",
            "lr", "pc",
        ];
        for (index, (name, value)) in NAMES.iter().zip(self.general).enumerate() {
            let separator = if index % 4 == 3 { "\n" } else { " " };
            write!(f, "{name:<3} {value:08X}{separator}")?;
        }
        let state = if self.thumb() { "Thumb" } else { "ARM" };
        write!(f, "cpsr {:08X} ({state})", self.cpsr)
    }
}

/// The access that fired a point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// A breakpoint reached its instruction.
    Execute,
    /// A watched address was read.
    Read {
        /// The value at the address.
        value: u32,
    },
    /// A watched address was written.
    Write {
        /// The value before the write.
        old: u32,
        /// The value written.
        new: u32,
    },
}

/// What a point saw when it fired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// The id of the point that fired.
    pub point: i64,
    /// The frame it fired in.
    pub frame: u32,
    /// The address that was accessed or reached.
    pub address: u32,
    /// The access that fired it.
    pub access: Access,
    /// The CPU's registers, where `pc` is the instruction that fired it.
    pub registers: Registers,
    /// The first [`MAX_STACK_WORDS`] words from the stack pointer up.
    pub stack: Vec<u32>,
}

/// The hits recorded while a command ran.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hits {
    /// The first hits, in the order they fired.
    pub kept: Vec<Hit>,
    /// How many hits fired, including those not kept.
    pub total: u32,
}

impl Hit {
    /// Describes the hit, showing `stack_words` words of the stack after a
    /// breakpoint's registers.
    #[must_use]
    pub fn describe(&self, stack_words: usize) -> String {
        let registers = &self.registers;
        match self.access {
            Access::Execute => {
                let mut text = format!(
                    "frame {}: break at {:08X}\n{registers}",
                    self.frame, self.address
                );
                let words = self.stack.iter().take(stack_words);
                for (line, chunk) in
                    (0_usize..).zip(words.collect::<Vec<_>>().chunks(STACK_WORDS_PER_LINE))
                {
                    let _ = write!(text, "\nsp+{}:", line * STACK_WORDS_PER_LINE * 4);
                    for word in chunk {
                        let _ = write!(text, " {word:08X}");
                    }
                }
                text
            }
            Access::Read { value } => format!(
                "frame {}: read {:08X} = {value:08X} {}",
                self.frame,
                self.address,
                self.context()
            ),
            Access::Write { old, new } => format!(
                "frame {}: write {:08X} {old:08X} -> {new:08X} {}",
                self.frame,
                self.address,
                self.context()
            ),
        }
    }

    /// Describes the instruction that made a watched access and the
    /// registers that usually hold its arguments.
    fn context(&self) -> String {
        let registers = &self.registers;
        let [r0, r1, r2, r3, ..] = registers.general;
        format!(
            "by {:08X}, lr {:08X}, r0 {r0:08X} r1 {r1:08X} r2 {r2:08X} r3 {r3:08X}",
            registers.pc(),
            registers.lr()
        )
    }
}

/// Describes the hits, showing as many stack words for each breakpoint hit
/// as its point in `points` asks for, then counts them.
#[must_use]
pub fn report(hits: &Hits, points: &[Point]) -> String {
    let mut lines: Vec<String> = hits
        .kept
        .iter()
        .map(|hit| {
            let stack = points
                .iter()
                .find(|point| point.id == hit.point)
                .map_or(0, |point| match point.kind {
                    PointKind::Break { stack } => stack as usize,
                    PointKind::Watch(_) => 0,
                });
            hit.describe(stack)
        })
        .collect();
    let kept = hits.kept.len();
    lines.push(match hits.total {
        0 => "No hits".to_string(),
        1 => "1 hit".to_string(),
        total if total as usize > kept => format!("{total} hits, listing the first {kept}"),
        total => format!("{total} hits"),
    });
    lines.join("\n")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Returns registers whose `rN` holds `0x0300_0000 + N`, in Thumb state.
    #[must_use]
    pub fn registers() -> Registers {
        let mut general = [0; 16];
        for (index, register) in (0..).zip(&mut general) {
            *register = 0x0300_0000 + index;
        }
        Registers {
            general,
            cpsr: 0x6000_003F,
        }
    }

    fn hit(point: i64, access: Access) -> Hit {
        Hit {
            point,
            frame: 15_980,
            address: 0x0200_316C,
            access,
            registers: registers(),
            stack: (0..MAX_STACK_WORDS).collect(),
        }
    }

    #[test]
    fn parse_watch_kinds() {
        assert_eq!("write".parse(), Ok(WatchKind::Write));
        assert_eq!("read".parse(), Ok(WatchKind::Read));
        assert_eq!("access".parse(), Ok(WatchKind::Access));
        assert_eq!("change".parse(), Ok(WatchKind::Change));
        assert_eq!(
            "exec".parse::<WatchKind>().unwrap_err().to_string(),
            "`exec` isn't a kind of watchpoint; expected write, read, access, or change"
        );
    }

    #[test]
    fn describe_points() {
        let watch = |kind| Point {
            id: 1,
            address: 0x0200_316C,
            kind: PointKind::Watch(kind),
        };
        assert_eq!(
            watch(WatchKind::Write).to_string(),
            "watching writes to 0x0200316C"
        );
        assert_eq!(
            watch(WatchKind::Read).to_string(),
            "watching reads to 0x0200316C"
        );
        assert_eq!(
            watch(WatchKind::Access).to_string(),
            "watching reads and writes to 0x0200316C"
        );
        assert_eq!(
            watch(WatchKind::Change).to_string(),
            "watching changes to 0x0200316C"
        );

        let mut point = Point {
            id: 2,
            address: 0x0804_FD08,
            kind: PointKind::Break { stack: 0 },
        };
        assert_eq!(point.to_string(), "breaking at 0x0804FD08");
        point.kind = PointKind::Break { stack: 48 };
        assert_eq!(
            point.to_string(),
            "breaking at 0x0804FD08 with 48 stack words"
        );
    }

    #[test]
    fn describe_registers() {
        let mut registers = registers();
        assert_eq!(
            registers.to_string(),
            "r0  03000000 r1  03000001 r2  03000002 r3  03000003\n\
             r4  03000004 r5  03000005 r6  03000006 r7  03000007\n\
             r8  03000008 r9  03000009 r10 0300000A r11 0300000B\n\
             r12 0300000C sp  0300000D lr  0300000E pc  0300000F\n\
             cpsr 6000003F (Thumb)"
        );
        assert_eq!(
            (registers.sp(), registers.lr(), registers.pc()),
            (0x0300_000D, 0x0300_000E, 0x0300_000F)
        );
        registers.cpsr = 0x1F;
        assert!(registers.to_string().ends_with("cpsr 0000001F (ARM)"));
    }

    #[test]
    fn describe_watch_hits() {
        assert_eq!(
            hit(
                1,
                Access::Write {
                    old: 0x118,
                    new: 0x111
                }
            )
            .describe(0),
            "frame 15980: write 0200316C 00000118 -> 00000111 by 0300000F, lr 0300000E, \
             r0 03000000 r1 03000001 r2 03000002 r3 03000003"
        );
        assert_eq!(
            hit(1, Access::Read { value: 0x118 }).describe(0),
            "frame 15980: read 0200316C = 00000118 by 0300000F, lr 0300000E, \
             r0 03000000 r1 03000001 r2 03000002 r3 03000003"
        );
    }

    #[test]
    fn describe_break_hits() {
        let hit = hit(2, Access::Execute);
        let without_stack = hit.describe(0);
        assert!(without_stack.starts_with("frame 15980: break at 0200316C\nr0  03000000 "));
        assert!(without_stack.ends_with("cpsr 6000003F (Thumb)"));

        let with_stack = hit.describe(10);
        assert!(with_stack.starts_with(&without_stack));
        assert!(with_stack.ends_with(
            "(Thumb)\nsp+0: 00000000 00000001 00000002 00000003 00000004 00000005 00000006 00000007\n\
             sp+32: 00000008 00000009"
        ));
    }

    #[test]
    fn report_hits() {
        let points = [
            Point {
                id: 1,
                address: 0x0200_316C,
                kind: PointKind::Watch(WatchKind::Write),
            },
            Point {
                id: 2,
                address: 0x0200_316C,
                kind: PointKind::Break { stack: 2 },
            },
        ];
        let write = hit(1, Access::Write { old: 1, new: 2 });
        let execute = hit(2, Access::Execute);
        assert_eq!(report(&Hits::default(), &points), "No hits");

        let one = Hits {
            kept: vec![write.clone()],
            total: 1,
        };
        assert_eq!(
            report(&one, &points),
            format!("{}\n1 hit", write.describe(0))
        );

        let many = Hits {
            kept: vec![execute.clone(), hit(3, Access::Execute)],
            total: 2,
        };
        assert_eq!(
            report(&many, &points),
            format!("{}\n{}\n2 hits", execute.describe(2), execute.describe(0))
        );

        let more = Hits {
            kept: vec![write],
            total: 75,
        };
        assert!(report(&more, &points).ends_with("\n75 hits, listing the first 1"));
    }
}
