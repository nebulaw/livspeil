// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The parameter expression language (kernel spec, section 6).
//!
//! Any parameter can hold an expression instead of a static value. The
//! language is tiny, statically typed and safe: no loops, no I/O and no
//! user-defined functions. An expression is [`compile`]d once into compact
//! bytecode with a known result type and then [`Program::eval`]uated many
//! times per frame. Purely numeric programs evaluate without allocating.
//!
//! # Grammar
//!
//! ```text
//! expr    = or ( "?" expr ":" expr )?             (right-associative)
//! or      = and ( "||" and )*
//! and     = eq ( "&&" eq )*
//! eq      = cmp ( ( "==" | "!=" ) cmp )*
//! cmp     = add ( ( "<" | "<=" | ">" | ">=" ) add )*
//! add     = mul ( ( "+" | "-" ) mul )*
//! mul     = unary ( ( "*" | "/" | "%" ) unary )*
//! unary   = ( "-" | "!" ) unary | power
//! power   = postfix ( "**" unary )?               (right-associative)
//! postfix = primary ( "." IDENT )*
//! primary = INT | FLOAT | STRING | "$" IDENT | IDENT
//!         | IDENT "(" ( expr ( "," expr )* )? ")" | "(" expr ")"
//! INT     = digit+                                e.g. 42
//! FLOAT   = digit+ ( "." digit+ )? ( [eE] [+-]? digit+ )?   with "." or exponent
//! STRING  = '"' ( char | '\"' | '\\' | '\n' | '\t' )* '"'
//! ```
//!
//! Note `-2 ** 2 == -(2 ** 2)` and `2 ** -1` is allowed.
//!
//! # Types and operators
//!
//! Types are `Float`, `Int`, `Bool`, `Vec2`, `Vec3`, `Vec4`, `Color` (OKLCH
//! plus alpha) and `String`. `Int` widens to `Float` implicitly wherever a
//! `Float` is needed; the reverse needs `int(x)`.
//!
//! * `+ - * /` on `Int` stay `Int` and are checked: overflow and division by
//!   zero are runtime errors. `/` and `%` on `Int` are *floored* (like
//!   Python: `-7 / 2 == -4`, `-7 % 2 == 1`), so `a == (a / b) * b + a % b`.
//!   Use `7 / 2.0` for float division.
//! * On `Float` everything follows IEEE 754: `1.0 / 0` is `inf`,
//!   `sqrt(-1)` is `NaN`. `%`/`mod` is floored (sign of the divisor).
//! * Vectors: component-wise `+ - * /` between vectors of the same size,
//!   `vector * scalar`, `scalar * vector`, `vector / scalar`, unary `-`.
//!   Components: `.x .y .z .w` and swizzles such as `.xy`, `.zyx`.
//! * Colors have no arithmetic; components are `.l .c .h .a`.
//! * `+` on two strings concatenates; `concat(a, b, ...)` takes any values and
//!   formats non-strings with `str()`.
//! * `< <= > >=` compare numbers; `== !=` compare any two values of the same
//!   type (numbers after widening, vectors and colors component-wise, IEEE
//!   equality so `NaN != NaN`). `&&` and `||` short-circuit; `!` negates.
//! * Conditionals: both `cond ? a : b` and `if(cond, a, b)` are supported and
//!   compile to the same code; only the chosen branch is evaluated. The
//!   branches must have the same type (`Int`/`Float` mix gives `Float`).
//!
//! # Variables, constants and functions
//!
//! `$F` (frame, `Int`), `$T` (seconds, `Float`), `$DT` (seconds since the
//! previous frame, `Float`); any of them makes the program time dependent.
//! Constants: `true`, `false`, `pi`, `tau`. `ch("path")` reads another
//! parameter; the path must be a string literal and is resolved at compile
//! time by an [`Env`].
//!
//! | Function | Arguments | Result |
//! |---|---|---|
//! | `sin cos tan asin acos atan sqrt exp ln log10 floor ceil round fract` | Float or vector | same (component-wise) |
//! | `abs sign` | Int, Float or vector | same |
//! | `atan2(y, x) pow(x, y) step(edge, x)` | Float | Float |
//! | `min max` | two Ints, two numbers or two same-size vectors | Int, Float or vector |
//! | `mod(a, b)` | two Ints or two numbers | floored modulo |
//! | `clamp(x, lo, hi)` | Ints, numbers, or vector + numbers | like `x` |
//! | `lerp(a, b, t)` | numbers, or two vectors and a number | like `a` |
//! | `smoothstep(e0, e1, x)` | Float | Float |
//! | `length normalize` / `dot cross` | vectors (`cross`: Vec3) | Float / vector |
//! | `vec2 vec3 vec4` / `oklch(l, c, h[, a])` | Float | vector / Color |
//! | `len upper lower` | String | Int / String |
//! | `str concat` / `int float` | any / number | String / Int, Float |
//!
//! Choices: `round` rounds half away from zero; `clamp` is
//! `min(max(x, lo), hi)` and never fails; `normalize` of a zero vector is the
//! zero vector; `smoothstep` with `e0 == e1` is a step; `int()` truncates
//! toward zero and saturates (`NaN` gives 0); `len` counts Unicode scalar
//! values; `upper`/`lower` use full Unicode case mapping.
//!
//! # Determinism
//!
//! Transcendental functions come from the pure-Rust `libm` crate, so results
//! are bit-identical on every platform. `str()` formats floats with Rust's
//! shortest round-trip representation (`1.0`, `0.1`, `1e100`, `NaN`, `inf`).

mod compile;
mod error;
mod lexer;
mod parser;
mod vm;

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use livspeil_node_api::{CookContext, PortType, Value};

pub use error::{ErrorKind, ExprError, Span};

/// Maximum nesting depth of an expression. Both the parser's recursion
/// (each parenthesis, prefix operator, call argument and right operand is a
/// level) and the height of the syntax tree are limited, so hostile input
/// cannot overflow the stack: at this limit compiling uses well under 1 MiB
/// of stack even in unoptimized builds.
pub const MAX_DEPTH: usize = 256;

/// Maximum length of an expression's source in bytes.
pub const MAX_SOURCE_LEN: usize = 64 * 1024;

/// A parameter that `ch("path")` resolved to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Channel {
    /// Index into [`EvalContext::slots`].
    pub slot: usize,
    /// The parameter's type; also the type of the `ch()` expression.
    pub ty: PortType,
}

/// Resolves `ch("path")` references while compiling.
pub trait Env {
    /// The slot and type of the parameter at `path`, or `None` if unknown.
    fn resolve(&self, path: &str) -> Option<Channel>;
}

/// An environment without parameters: every `ch()` fails to compile.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoChannels;

impl Env for NoChannels {
    fn resolve(&self, _path: &str) -> Option<Channel> {
        None
    }
}

impl Env for BTreeMap<String, Channel> {
    fn resolve(&self, path: &str) -> Option<Channel> {
        self.get(path).copied()
    }
}

impl<S: std::hash::BuildHasher> Env for HashMap<String, Channel, S> {
    fn resolve(&self, path: &str) -> Option<Channel> {
        self.get(path).copied()
    }
}

/// Everything an expression can read while evaluating.
#[derive(Debug, Clone, Copy, Default)]
pub struct EvalContext<'a> {
    /// `$F`.
    pub frame: i64,
    /// `$T`, seconds.
    pub time: f64,
    /// `$DT`, seconds.
    pub dt: f64,
    /// Values of the parameters `ch()` reads, indexed by [`Channel::slot`].
    pub slots: &'a [Value],
}

impl<'a> EvalContext<'a> {
    /// A context with the given clock and slots.
    #[must_use]
    pub fn new(frame: i64, time: f64, dt: f64, slots: &'a [Value]) -> Self {
        EvalContext {
            frame,
            time,
            dt,
            slots,
        }
    }

    /// A context from the cook clock. Frames beyond `i64::MAX` saturate.
    #[must_use]
    pub fn from_cook(cook: &CookContext, slots: &'a [Value]) -> Self {
        EvalContext {
            frame: i64::try_from(cook.frame).unwrap_or(i64::MAX),
            time: cook.time,
            dt: cook.dt,
            slots,
        }
    }
}

/// A compiled expression.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    code: Vec<vm::Op>,
    /// Source span of each instruction, for runtime errors.
    spans: Vec<Span>,
    strings: Vec<String>,
    ty: PortType,
    deps: Vec<usize>,
    time_dependent: bool,
    /// Peak depth of the numeric stack.
    max_num: usize,
}

impl Program {
    /// Evaluate the program.
    ///
    /// # Errors
    /// Returns a runtime [`ExprError`] (integer division by zero or overflow,
    /// a missing slot or a slot of the wrong type) with the span of the
    /// offending operation.
    pub fn eval(&self, ctx: &EvalContext<'_>) -> Result<Value, ExprError> {
        vm::run(self, ctx)
    }

    /// The type of every value [`Program::eval`] returns.
    #[must_use]
    pub fn result_type(&self) -> PortType {
        self.ty
    }

    /// The slots read through `ch()`, sorted and without duplicates. Slots in
    /// branches that may not run are included.
    #[must_use]
    pub fn dependencies(&self) -> &[usize] {
        &self.deps
    }

    /// Whether the program reads `$F`, `$T` or `$DT`.
    #[must_use]
    pub fn is_time_dependent(&self) -> bool {
        self.time_dependent
    }

    /// Number of bytecode instructions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.code.len()
    }

    /// Whether the program has no instructions (never true for a compiled
    /// program).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.code.is_empty()
    }
}

/// Compile `source`, resolving `ch()` paths through `env`.
///
/// # Errors
/// Returns an [`ExprError`] with a byte span for syntax errors, unknown
/// names, wrong arity, type mismatches and unknown `ch()` paths.
pub fn compile(source: &str, env: &dyn Env) -> Result<Program, ExprError> {
    if source.len() > MAX_SOURCE_LEN {
        return Err(ExprError::new(
            ErrorKind::SourceTooLong {
                limit: MAX_SOURCE_LEN,
            },
            Span::new(0, source.len()),
        ));
    }
    let ast = parser::parse(source).map_err(|e| *e)?;
    compile::Gen::new(env).finish(&ast)
}

/// Deterministic text form of a value, as produced by `str()`.
///
/// Floats use the shortest representation that round-trips (`1.0`, `0.1`,
/// `1e100`), Ints and Bools the obvious one, vectors `vec3(1.0, 2.0, 3.0)`,
/// colors `oklch(l, c, h, a)`; strings are returned unchanged.
#[must_use]
pub fn format_value(value: &Value) -> String {
    fn list(name: &str, items: &[f64]) -> String {
        let mut out = format!("{name}(");
        for (i, x) in items.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            let _ = write!(out, "{x:?}");
        }
        out.push(')');
        out
    }
    match value {
        Value::Float(x) => format!("{x:?}"),
        Value::Int(i) => i.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Vec2(v) => list("vec2", v),
        Value::Vec3(v) => list("vec3", v),
        Value::Vec4(v) => list("vec4", v),
        Value::Color(v) => list("oklch", v),
        Value::String(s) => s.clone(),
        other => format!("{other:?}"),
    }
}
