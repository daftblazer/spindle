// SPDX-License-Identifier: GPL-3.0-or-later

//! HDMV navigation command encoder (12-byte instructions used by movie
//! objects and IG button commands).

use crate::bluray::bytes::BitWriter;

/// Instruction operand.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operand {
    Imm(u32),
    /// General purpose register (0..4095).
    Gpr(u16),
    /// Player status register (0..127).
    Psr(u8),
}

impl Operand {
    fn is_imm(self) -> bool {
        matches!(self, Operand::Imm(_))
    }

    fn value(self) -> u32 {
        match self {
            Operand::Imm(v) => v,
            Operand::Gpr(r) => r as u32,
            Operand::Psr(r) => 0x8000_0000 | r as u32,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmp {
    Eq,
    Ne,
    Ge,
    Gt,
    Le,
    Lt,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Nop,
    Goto(u32),
    Break,
    JumpObject(u32),
    JumpTitle(u32),
    CallTitle(u32),
    Resume,
    PlayPl(Operand),
    PlayPlAtMark(Operand, Operand),
    LinkMark(Operand),
    TerminatePl,
    /// Skip the next instruction unless `a <cmp> b` holds.
    Compare(Cmp, Operand, Operand),
    /// `gpr = src`
    Move(u16, Operand),
    /// Select PG (subtitle) stream `number` (1-based) and turn its display
    /// on or off.
    SetPgStream { number: u16, display: bool },
    /// Set selected button (`button`) and/or page (`page`).
    SetButtonPage { button: Option<u16>, page: Option<u8> },
}

const GRP_BRANCH: u8 = 0;
const GRP_CMP: u8 = 1;
const GRP_SET: u8 = 2;

struct Raw {
    op_cnt: u8,
    grp: u8,
    sub_grp: u8,
    imm1: bool,
    imm2: bool,
    branch_opt: u8,
    cmp_opt: u8,
    set_opt: u8,
    dst: u32,
    src: u32,
}

impl Raw {
    fn branch(sub_grp: u8, opt: u8, ops: &[Operand]) -> Self {
        let mut r = Raw {
            op_cnt: ops.len() as u8,
            grp: GRP_BRANCH,
            sub_grp,
            imm1: false,
            imm2: false,
            branch_opt: opt,
            cmp_opt: 0,
            set_opt: 0,
            dst: 0,
            src: 0,
        };
        r.set_ops(ops);
        r
    }

    fn set_ops(&mut self, ops: &[Operand]) {
        if let Some(o) = ops.first() {
            self.imm1 = o.is_imm();
            self.dst = o.value();
        }
        if let Some(o) = ops.get(1) {
            self.imm2 = o.is_imm();
            self.src = o.value();
        }
    }
}

impl Command {
    fn raw(self) -> Raw {
        use Operand::Imm;
        match self {
            Command::Nop => Raw::branch(0, 0, &[]),
            Command::Goto(line) => Raw::branch(0, 1, &[Imm(line)]),
            Command::Break => Raw::branch(0, 2, &[]),
            Command::JumpObject(o) => Raw::branch(1, 0, &[Imm(o)]),
            Command::JumpTitle(t) => Raw::branch(1, 1, &[Imm(t)]),
            Command::CallTitle(t) => Raw::branch(1, 3, &[Imm(t)]),
            Command::Resume => Raw::branch(1, 4, &[]),
            Command::PlayPl(pl) => Raw::branch(2, 0, &[pl]),
            Command::PlayPlAtMark(pl, mark) => Raw::branch(2, 2, &[pl, mark]),
            Command::TerminatePl => Raw::branch(2, 3, &[]),
            Command::LinkMark(mark) => Raw::branch(2, 5, &[mark]),
            Command::Compare(cmp, a, b) => {
                let mut r = Raw::branch(0, 0, &[a, b]);
                r.grp = GRP_CMP;
                r.cmp_opt = match cmp {
                    Cmp::Eq => 2,
                    Cmp::Ne => 3,
                    Cmp::Ge => 4,
                    Cmp::Gt => 5,
                    Cmp::Le => 6,
                    Cmp::Lt => 7,
                };
                r
            }
            Command::Move(dst, src) => {
                let mut r = Raw::branch(0, 0, &[Operand::Gpr(dst), src]);
                r.grp = GRP_SET;
                r.set_opt = 1;
                r
            }
            Command::SetPgStream { number, display } => {
                let dst = 0x8000 | ((display as u32) << 14) | (number as u32 & 0xFFF);
                let mut r = Raw::branch(0, 0, &[Imm(dst), Imm(0)]);
                r.grp = GRP_SET;
                r.sub_grp = 1;
                r.set_opt = 1;
                r
            }
            Command::SetButtonPage { button, page } => {
                // dst: bit31 = button valid, low 16 bits button id
                // src: bit31 = page valid, low 8 bits page id
                let dst = button.map_or(0, |b| 0x8000_0000 | b as u32);
                let src = page.map_or(0, |p| 0x8000_0000 | p as u32);
                let mut r = Raw::branch(0, 0, &[Imm(dst), Imm(src)]);
                r.grp = GRP_SET;
                r.sub_grp = 1;
                r.set_opt = 3;
                r
            }
        }
    }

    pub fn write(self, w: &mut BitWriter) {
        let r = self.raw();
        w.bits(3, r.op_cnt as u64)
            .bits(2, r.grp as u64)
            .bits(3, r.sub_grp as u64)
            .flag(r.imm1)
            .flag(r.imm2)
            .zeros(2)
            .bits(4, r.branch_opt as u64)
            .zeros(4)
            .bits(4, r.cmp_opt as u64)
            .zeros(3)
            .bits(5, r.set_opt as u64)
            .u32(r.dst)
            .u32(r.src);
    }

    #[cfg(test)]
    pub fn to_bytes(self) -> [u8; 12] {
        let mut w = BitWriter::new();
        self.write(&mut w);
        w.into_bytes().try_into().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_pl_encoding() {
        // "PlayPL 1" as found on commercial discs: 22 80 00 00 00000001 00000000
        assert_eq!(
            Command::PlayPl(Operand::Imm(1)).to_bytes(),
            [0x22, 0x80, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0]
        );
    }

    #[test]
    fn jump_title_encoding() {
        // "JumpTitle 2": 21 81 00 00 00000002 00000000
        assert_eq!(
            Command::JumpTitle(2).to_bytes(),
            [0x21, 0x81, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0]
        );
    }

    #[test]
    fn move_encoding() {
        // "Move GPR0, 5": 50 40 00 01 00000000 00000005
        assert_eq!(
            Command::Move(0, Operand::Imm(5)).to_bytes(),
            [0x50, 0x40, 0, 1, 0, 0, 0, 0, 0, 0, 0, 5]
        );
    }
}
