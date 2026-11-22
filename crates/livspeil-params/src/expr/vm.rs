// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Bytecode and the stack machine that runs it.
//!
//! Programs are statically typed, so every instruction knows the types of
//! its operands. Numeric values live on a stack of fixed-size [`Cell`]s
//! (kept on the native stack for small programs, so purely numeric
//! evaluation does not allocate); strings live on a separate stack.

use livspeil_node_api::{PortType, Value};

use super::error::{ErrorKind, ExprError, Span};
use super::{EvalContext, Program, format_value};

/// One numeric stack slot. Ints use `i`, bools use `i` (0 or 1), floats use
/// `v[0]`, vectors and colors use `v`.
#[derive(Debug, Clone, Copy, Default)]
struct Cell {
    i: i64,
    v: [f64; 4],
}

impl Cell {
    fn float(x: f64) -> Cell {
        Cell {
            i: 0,
            v: [x, 0.0, 0.0, 0.0],
        }
    }
    fn int(i: i64) -> Cell {
        Cell { i, v: [0.0; 4] }
    }
    fn bool(b: bool) -> Cell {
        Cell::int(i64::from(b))
    }
    fn vec(v: [f64; 4]) -> Cell {
        Cell { i: 0, v }
    }
}

/// Float functions of one argument (also applied component-wise to vectors).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum F1 {
    Neg,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sqrt,
    Exp,
    Ln,
    Log10,
    Abs,
    Floor,
    Ceil,
    Round,
    Fract,
    Sign,
}

impl F1 {
    fn apply(self, x: f64) -> f64 {
        match self {
            F1::Neg => -x,
            F1::Sin => libm::sin(x),
            F1::Cos => libm::cos(x),
            F1::Tan => libm::tan(x),
            F1::Asin => libm::asin(x),
            F1::Acos => libm::acos(x),
            F1::Atan => libm::atan(x),
            F1::Sqrt => x.sqrt(),
            F1::Exp => libm::exp(x),
            F1::Ln => libm::log(x),
            F1::Log10 => libm::log10(x),
            F1::Abs => x.abs(),
            F1::Floor => x.floor(),
            F1::Ceil => x.ceil(),
            F1::Round => x.round(),
            F1::Fract => x - x.floor(),
            F1::Sign => {
                if x > 0.0 {
                    1.0
                } else if x < 0.0 {
                    -1.0
                } else {
                    x // keeps +0, -0 and NaN
                }
            }
        }
    }
}

/// Float functions of two arguments (also component-wise on vectors).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum F2 {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Atan2,
    Min,
    Max,
    Step,
}

impl F2 {
    fn apply(self, a: f64, b: f64) -> f64 {
        match self {
            F2::Add => a + b,
            F2::Sub => a - b,
            F2::Mul => a * b,
            F2::Div => a / b,
            F2::Mod => {
                // Floored modulo: the result has the sign of the divisor.
                let r = libm::fmod(a, b);
                if r != 0.0 && ((r < 0.0) != (b < 0.0)) {
                    r + b
                } else {
                    r
                }
            }
            F2::Pow => libm::pow(a, b),
            F2::Atan2 => libm::atan2(a, b),
            F2::Min => a.min(b),
            F2::Max => a.max(b),
            // step(edge, x)
            F2::Step => {
                if b < a {
                    0.0
                } else {
                    1.0
                }
            }
        }
    }
}

/// Float functions of three arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum F3 {
    Clamp,
    Lerp,
    Smoothstep,
}

fn clamp(x: f64, lo: f64, hi: f64) -> f64 {
    // Never panics, unlike f64::clamp; NaN bounds are ignored by min/max.
    x.max(lo).min(hi)
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

impl F3 {
    fn apply(self, a: f64, b: f64, c: f64) -> f64 {
        match self {
            F3::Clamp => clamp(a, b, c),
            F3::Lerp => lerp(a, b, c),
            F3::Smoothstep => {
                let t = clamp((c - a) / (b - a), 0.0, 1.0);
                t * t * (3.0 - 2.0 * t)
            }
        }
    }
}

/// Comparison operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

impl Cmp {
    fn apply<T: PartialOrd>(self, a: T, b: T) -> bool {
        match self {
            Cmp::Lt => a < b,
            Cmp::Le => a <= b,
            Cmp::Gt => a > b,
            Cmp::Ge => a >= b,
            Cmp::Eq => a == b,
            Cmp::Ne => a != b,
        }
    }
}

/// Checked integer binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum I2 {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Min,
    Max,
}

impl I2 {
    fn apply(self, a: i64, b: i64) -> Result<i64, ErrorKind> {
        const OVF: ErrorKind = ErrorKind::IntegerOverflow;
        match self {
            I2::Add => a.checked_add(b).ok_or(OVF),
            I2::Sub => a.checked_sub(b).ok_or(OVF),
            I2::Mul => a.checked_mul(b).ok_or(OVF),
            I2::Div | I2::Mod => {
                if b == 0 {
                    return Err(ErrorKind::DivisionByZero);
                }
                let q = a.checked_div(b).ok_or(OVF)?;
                let r = a.checked_rem(b).ok_or(OVF)?;
                // Floored: adjust when the remainder and divisor differ in sign.
                let adjust = r != 0 && ((r < 0) != (b < 0));
                if self == I2::Div {
                    Ok(if adjust { q - 1 } else { q })
                } else {
                    Ok(if adjust { r + b } else { r })
                }
            }
            I2::Min => Ok(a.min(b)),
            I2::Max => Ok(a.max(b)),
        }
    }
}

/// One instruction. Vector instructions carry their component count `n`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Op {
    Nop,
    PushF(f64),
    PushI(i64),
    PushB(bool),
    /// Push string constant `strings[i]`.
    PushS(usize),
    /// Push the value of `ch()` slot `slot`, typed `ty`.
    Load(usize, PortType),
    Frame,
    Time,
    Dt,
    /// Convert the Int `depth` cells below the top to Float.
    IntToFloat(u32),
    /// Float to Int: truncate toward zero, saturate, NaN becomes 0.
    FloatToInt,
    Jump(usize),
    /// Pop a Bool and jump if it is false.
    JumpIfFalse(usize),
    I2(I2),
    NegI,
    AbsI,
    SignI,
    ClampI,
    F1(F1),
    F2(F2),
    F3(F3),
    /// Component-wise unary on an `n`-vector.
    V1(F1, u8),
    /// Component-wise binary on two `n`-vectors.
    V2(F2, u8),
    /// `vector op scalar`.
    VS(F2, u8),
    /// `scalar op vector`.
    SV(F2, u8),
    /// `clamp(vector, scalar, scalar)`.
    VClamp(u8),
    /// `lerp(vector, vector, scalar)`.
    VLerp(u8),
    Dot(u8),
    Length(u8),
    Normalize(u8),
    Cross,
    /// Pop `n` floats, push an `n`-vector.
    MakeVec(u8),
    /// Pop l, c, h (and alpha when `true`), push a color.
    MakeColor(bool),
    /// Vector or color component `i` as Float.
    Comp(u8),
    /// Swizzle into an `n`-vector.
    Swizzle([u8; 4], u8),
    CmpI(Cmp),
    CmpF(Cmp),
    /// Bool equality; `true` negates (`!=`).
    EqB(bool),
    /// Vector/color equality over `n` components; `true` negates.
    EqV(u8, bool),
    /// String equality; `true` negates.
    EqS(bool),
    Not,
    Concat,
    Len,
    Upper,
    Lower,
    /// Pop a numeric value of the given type, push its string form.
    StrOf(PortType),
}

impl Op {
    /// `(numeric stack delta, string stack delta)` of this instruction.
    pub(crate) fn effect(self) -> (isize, isize) {
        match self {
            Op::Nop
            | Op::IntToFloat(_)
            | Op::FloatToInt
            | Op::Jump(_)
            | Op::NegI
            | Op::AbsI
            | Op::SignI
            | Op::F1(_)
            | Op::V1(..)
            | Op::Length(_)
            | Op::Normalize(_)
            | Op::Comp(_)
            | Op::Swizzle(..)
            | Op::Not => (0, 0),
            Op::PushF(_) | Op::PushI(_) | Op::PushB(_) | Op::Frame | Op::Time | Op::Dt => (1, 0),
            Op::PushS(_) => (0, 1),
            Op::Load(_, PortType::String) => (0, 1),
            Op::Load(..) => (1, 0),
            Op::JumpIfFalse(_)
            | Op::I2(_)
            | Op::F2(_)
            | Op::V2(..)
            | Op::VS(..)
            | Op::SV(..)
            | Op::Dot(_)
            | Op::Cross
            | Op::CmpI(_)
            | Op::CmpF(_)
            | Op::EqB(_)
            | Op::EqV(..) => (-1, 0),
            Op::ClampI | Op::F3(_) | Op::VClamp(_) | Op::VLerp(_) => (-2, 0),
            Op::MakeVec(n) => (1 - isize::from(n), 0),
            Op::MakeColor(alpha) => (if alpha { -3 } else { -2 }, 0),
            Op::EqS(_) => (1, -2),
            Op::Concat => (0, -1),
            Op::Len => (1, -1),
            Op::Upper | Op::Lower => (0, 0),
            Op::StrOf(_) => (-1, 1),
        }
    }
}

/// Number of cells kept on the native stack before falling back to the heap.
const INLINE_CELLS: usize = 16;

const STACK_BUG: ErrorKind = ErrorKind::Internal("stack underflow or overflow");

struct Machine<'s> {
    num: &'s mut [Cell],
    sp: usize,
    strs: Vec<String>,
}

impl Machine<'_> {
    fn push(&mut self, c: Cell) -> Result<(), ErrorKind> {
        let slot = self.num.get_mut(self.sp).ok_or(STACK_BUG)?;
        *slot = c;
        self.sp += 1;
        Ok(())
    }

    fn pop(&mut self) -> Result<Cell, ErrorKind> {
        self.sp = self.sp.checked_sub(1).ok_or(STACK_BUG)?;
        self.num.get(self.sp).copied().ok_or(STACK_BUG)
    }

    fn top(&mut self) -> Result<&mut Cell, ErrorKind> {
        let i = self.sp.checked_sub(1).ok_or(STACK_BUG)?;
        self.num.get_mut(i).ok_or(STACK_BUG)
    }

    fn pop_str(&mut self) -> Result<String, ErrorKind> {
        self.strs.pop().ok_or(STACK_BUG)
    }

    fn top_str(&mut self) -> Result<&mut String, ErrorKind> {
        self.strs.last_mut().ok_or(STACK_BUG)
    }
}

fn map_n(n: u8, mut f: impl FnMut(usize) -> f64) -> [f64; 4] {
    let mut out = [0.0; 4];
    for (i, o) in out.iter_mut().enumerate().take(usize::from(n)) {
        *o = f(i);
    }
    out
}

fn dot(n: u8, a: &[f64; 4], b: &[f64; 4]) -> f64 {
    a.iter()
        .zip(b)
        .take(usize::from(n))
        .fold(0.0, |s, (x, y)| s + x * y)
}

/// Convert a numeric cell of type `ty` to a value.
fn cell_value(c: Cell, ty: PortType) -> Result<Value, ErrorKind> {
    let v = c.v;
    Ok(match ty {
        PortType::Float => Value::Float(v[0]),
        PortType::Int => Value::Int(c.i),
        PortType::Bool => Value::Bool(c.i != 0),
        PortType::Vec2 => Value::Vec2([v[0], v[1]]),
        PortType::Vec3 => Value::Vec3([v[0], v[1], v[2]]),
        PortType::Vec4 => Value::Vec4(v),
        PortType::Color => Value::Color(v),
        _ => return Err(ErrorKind::Internal("non-numeric cell type")),
    })
}

/// Run `p` against `ctx`.
pub(crate) fn run(p: &Program, ctx: &EvalContext<'_>) -> Result<Value, ExprError> {
    let mut inline = [Cell::default(); INLINE_CELLS];
    let mut heap = Vec::new();
    let num: &mut [Cell] = if p.max_num <= INLINE_CELLS {
        &mut inline
    } else {
        heap.resize(p.max_num, Cell::default());
        &mut heap
    };
    let mut m = Machine {
        num,
        sp: 0,
        strs: Vec::new(),
    };
    let mut pc = 0;
    exec(p, ctx, &mut m, &mut pc).map_err(|kind| {
        let span = p.spans.get(pc).copied().unwrap_or(Span::new(0, 0));
        ExprError::new(kind, span)
    })
}

fn exec(
    p: &Program,
    ctx: &EvalContext<'_>,
    m: &mut Machine<'_>,
    pc: &mut usize,
) -> Result<Value, ErrorKind> {
    while let Some(&op) = p.code.get(*pc) {
        let mut next = *pc + 1;
        match op {
            Op::Nop => {}
            Op::PushF(x) => m.push(Cell::float(x))?,
            Op::PushI(i) => m.push(Cell::int(i))?,
            Op::PushB(b) => m.push(Cell::bool(b))?,
            Op::PushS(i) => {
                let s = p.strings.get(i).ok_or(ErrorKind::Internal("bad string"))?;
                m.strs.push(s.clone());
            }
            Op::Load(slot, ty) => load(m, ctx, slot, ty)?,
            Op::Frame => m.push(Cell::int(ctx.frame))?,
            Op::Time => m.push(Cell::float(ctx.time))?,
            Op::Dt => m.push(Cell::float(ctx.dt))?,
            Op::IntToFloat(depth) => {
                let idx = m.sp.checked_sub(1 + depth as usize).ok_or(STACK_BUG)?;
                let c = m.num.get_mut(idx).ok_or(STACK_BUG)?;
                *c = Cell::float(c.i as f64);
            }
            Op::FloatToInt => {
                let c = m.top()?;
                *c = Cell::int(c.v[0] as i64);
            }
            Op::Jump(t) => next = t,
            Op::JumpIfFalse(t) => {
                if m.pop()?.i == 0 {
                    next = t;
                }
            }
            Op::I2(f) => {
                let b = m.pop()?.i;
                let a = m.top()?;
                a.i = f.apply(a.i, b)?;
            }
            Op::NegI => {
                let a = m.top()?;
                a.i = a.i.checked_neg().ok_or(ErrorKind::IntegerOverflow)?;
            }
            Op::AbsI => {
                let a = m.top()?;
                a.i = a.i.checked_abs().ok_or(ErrorKind::IntegerOverflow)?;
            }
            Op::SignI => {
                let a = m.top()?;
                a.i = a.i.signum();
            }
            Op::ClampI => {
                let hi = m.pop()?.i;
                let lo = m.pop()?.i;
                let a = m.top()?;
                // Never panics, unlike Ord::clamp.
                a.i = a.i.max(lo).min(hi);
            }
            Op::F1(f) => {
                let a = m.top()?;
                a.v[0] = f.apply(a.v[0]);
            }
            Op::F2(f) => {
                let b = m.pop()?.v[0];
                let a = m.top()?;
                a.v[0] = f.apply(a.v[0], b);
            }
            Op::F3(f) => {
                let c = m.pop()?.v[0];
                let b = m.pop()?.v[0];
                let a = m.top()?;
                a.v[0] = f.apply(a.v[0], b, c);
            }
            Op::V1(f, n) => {
                let a = m.top()?;
                a.v = map_n(n, |i| f.apply(a.v[i]));
            }
            Op::V2(f, n) => {
                let b = m.pop()?.v;
                let a = m.top()?;
                a.v = map_n(n, |i| f.apply(a.v[i], b[i]));
            }
            Op::VS(f, n) => {
                let s = m.pop()?.v[0];
                let a = m.top()?;
                a.v = map_n(n, |i| f.apply(a.v[i], s));
            }
            Op::SV(f, n) => {
                let b = m.pop()?.v;
                let a = m.top()?;
                let s = a.v[0];
                a.v = map_n(n, |i| f.apply(s, b[i]));
            }
            Op::VClamp(n) => {
                let hi = m.pop()?.v[0];
                let lo = m.pop()?.v[0];
                let a = m.top()?;
                a.v = map_n(n, |i| clamp(a.v[i], lo, hi));
            }
            Op::VLerp(n) => {
                let t = m.pop()?.v[0];
                let b = m.pop()?.v;
                let a = m.top()?;
                a.v = map_n(n, |i| lerp(a.v[i], b[i], t));
            }
            Op::Dot(n) => {
                let b = m.pop()?.v;
                let a = m.top()?;
                *a = Cell::float(dot(n, &a.v, &b));
            }
            Op::Length(n) => {
                let a = m.top()?;
                *a = Cell::float(dot(n, &a.v, &a.v).sqrt());
            }
            Op::Normalize(n) => {
                let a = m.top()?;
                let len = dot(n, &a.v, &a.v).sqrt();
                // The zero vector normalizes to itself instead of NaN.
                if len > 0.0 {
                    a.v = map_n(n, |i| a.v[i] / len);
                }
            }
            Op::Cross => {
                let b = m.pop()?.v;
                let a = m.top()?;
                let v = a.v;
                a.v = [
                    v[1] * b[2] - v[2] * b[1],
                    v[2] * b[0] - v[0] * b[2],
                    v[0] * b[1] - v[1] * b[0],
                    0.0,
                ];
            }
            Op::MakeVec(n) => {
                let mut out = [0.0; 4];
                for o in out.iter_mut().take(usize::from(n)).rev() {
                    *o = m.pop()?.v[0];
                }
                m.push(Cell::vec(out))?;
            }
            Op::MakeColor(alpha) => {
                let a = if alpha { m.pop()?.v[0] } else { 1.0 };
                let h = m.pop()?.v[0];
                let c = m.pop()?.v[0];
                let l = m.pop()?.v[0];
                m.push(Cell::vec([l, c, h, a]))?;
            }
            Op::Comp(i) => {
                let a = m.top()?;
                let x = a.v.get(usize::from(i)).copied().ok_or(STACK_BUG)?;
                *a = Cell::float(x);
            }
            Op::Swizzle(idx, n) => {
                let a = m.top()?;
                let src = a.v;
                a.v = map_n(n, |i| src[usize::from(idx[i] & 3)]);
            }
            Op::CmpI(c) => {
                let b = m.pop()?.i;
                let a = m.top()?;
                *a = Cell::bool(c.apply(a.i, b));
            }
            Op::CmpF(c) => {
                let b = m.pop()?.v[0];
                let a = m.top()?;
                *a = Cell::bool(c.apply(a.v[0], b));
            }
            Op::EqB(neg) => {
                let b = m.pop()?.i;
                let a = m.top()?;
                *a = Cell::bool((a.i == b) != neg);
            }
            Op::EqV(n, neg) => {
                let b = m.pop()?.v;
                let a = m.top()?;
                let n = usize::from(n).min(4);
                *a = Cell::bool((a.v[..n] == b[..n]) != neg);
            }
            Op::EqS(neg) => {
                let b = m.pop_str()?;
                let a = m.pop_str()?;
                m.push(Cell::bool((a == b) != neg))?;
            }
            Op::Not => {
                let a = m.top()?;
                a.i = i64::from(a.i == 0);
            }
            Op::Concat => {
                let b = m.pop_str()?;
                m.top_str()?.push_str(&b);
            }
            Op::Len => {
                let s = m.pop_str()?;
                let n = i64::try_from(s.chars().count()).unwrap_or(i64::MAX);
                m.push(Cell::int(n))?;
            }
            Op::Upper => {
                let s = m.top_str()?;
                *s = s.to_uppercase();
            }
            Op::Lower => {
                let s = m.top_str()?;
                *s = s.to_lowercase();
            }
            Op::StrOf(ty) => {
                let c = m.pop()?;
                let s = format_value(&cell_value(c, ty)?);
                m.strs.push(s);
            }
        }
        *pc = next;
    }
    if p.ty == PortType::String {
        Ok(Value::String(m.pop_str()?))
    } else {
        cell_value(m.pop()?, p.ty)
    }
}

fn load(
    m: &mut Machine<'_>,
    ctx: &EvalContext<'_>,
    slot: usize,
    ty: PortType,
) -> Result<(), ErrorKind> {
    let value = ctx.slots.get(slot).ok_or(ErrorKind::MissingSlot { slot })?;
    let cell = match (value, ty) {
        (Value::Float(x), PortType::Float) => Cell::float(*x),
        (Value::Int(i), PortType::Float) => Cell::float(*i as f64),
        (Value::Int(i), PortType::Int) => Cell::int(*i),
        (Value::Bool(b), PortType::Bool) => Cell::bool(*b),
        (Value::Vec2(v), PortType::Vec2) => Cell::vec([v[0], v[1], 0.0, 0.0]),
        (Value::Vec3(v), PortType::Vec3) => Cell::vec([v[0], v[1], v[2], 0.0]),
        (Value::Vec4(v), PortType::Vec4) | (Value::Color(v), PortType::Color) => Cell::vec(*v),
        (Value::String(s), PortType::String) => {
            m.strs.push(s.clone());
            return Ok(());
        }
        (other, expected) => {
            return Err(ErrorKind::SlotTypeMismatch {
                slot,
                expected,
                found: other.port_type(),
            });
        }
    };
    m.push(cell)
}
