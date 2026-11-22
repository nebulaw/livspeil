// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Errors of the expression language: parse, type and runtime errors.

use std::fmt;

use livspeil_node_api::PortType;

/// A half-open byte range `start..end` into the expression source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    /// A span covering `start..end`.
    #[must_use]
    pub const fn new(start: usize, end: usize) -> Self {
        Span { start, end }
    }

    /// The smallest span covering both `self` and `other`.
    #[must_use]
    pub fn to(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

/// What went wrong. Grouped into parse, compile (type) and runtime errors.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    // ---- Lexing and parsing ----------------------------------------------
    /// The source is longer than [`super::MAX_SOURCE_LEN`] bytes.
    SourceTooLong { limit: usize },
    /// A character that cannot start any token.
    UnexpectedChar(char),
    /// A string literal without its closing quote.
    UnterminatedString,
    /// An unknown escape sequence such as `\q` inside a string literal.
    InvalidEscape(char),
    /// A malformed or out-of-range number literal.
    InvalidNumber(&'static str),
    /// The parser wanted one thing and found another.
    Expected {
        expected: &'static str,
        found: String,
    },
    /// Nesting deeper than [`super::MAX_DEPTH`].
    TooDeep { limit: usize },

    // ---- Name resolution and type checking ---------------------------------
    /// A call to a function that does not exist.
    UnknownFunction(String),
    /// A `$NAME` that is not a known variable.
    UnknownVariable(String),
    /// A bare identifier that is not a known constant.
    UnknownIdentifier(String),
    /// A `.component` that the value does not have.
    UnknownComponent { component: String, ty: PortType },
    /// Wrong number of arguments.
    Arity {
        function: String,
        expected: &'static str,
        found: usize,
    },
    /// An operand or argument has the wrong type.
    TypeMismatch {
        context: String,
        expected: String,
        found: PortType,
    },
    /// `ch("path")` names a parameter the environment does not know.
    UnknownChannel(String),
    /// `ch()` needs exactly one string literal so dependencies are static.
    ChannelPathNotLiteral,
    /// `ch("path")` resolved to a type expressions cannot hold.
    UnsupportedChannelType { path: String, ty: PortType },

    // ---- Runtime -----------------------------------------------------------
    /// Integer division or modulo by zero.
    DivisionByZero,
    /// Integer arithmetic overflowed `i64`.
    IntegerOverflow,
    /// The evaluation context has no value for a slot read by `ch()`.
    MissingSlot { slot: usize },
    /// The evaluation context holds a value of the wrong type for a slot.
    SlotTypeMismatch {
        slot: usize,
        expected: PortType,
        found: PortType,
    },
    /// A bug in the compiler or virtual machine; never expected.
    Internal(&'static str),
}

impl ErrorKind {
    /// Whether this error can only happen while evaluating.
    #[must_use]
    pub fn is_runtime(&self) -> bool {
        matches!(
            self,
            ErrorKind::DivisionByZero
                | ErrorKind::IntegerOverflow
                | ErrorKind::MissingSlot { .. }
                | ErrorKind::SlotTypeMismatch { .. }
                | ErrorKind::Internal(_)
        )
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorKind::SourceTooLong { limit } => {
                write!(f, "expression is longer than {limit} bytes")
            }
            ErrorKind::UnexpectedChar(c) => write!(f, "unexpected character {c:?}"),
            ErrorKind::UnterminatedString => f.write_str("unterminated string literal"),
            ErrorKind::InvalidEscape(c) => write!(f, "invalid escape sequence '\\{c}'"),
            ErrorKind::InvalidNumber(why) => write!(f, "invalid number: {why}"),
            ErrorKind::Expected { expected, found } => {
                write!(f, "expected {expected}, found {found}")
            }
            ErrorKind::TooDeep { limit } => {
                write!(f, "expression is nested too deeply (limit {limit})")
            }
            ErrorKind::UnknownFunction(name) => write!(f, "unknown function '{name}'"),
            ErrorKind::UnknownVariable(name) => {
                write!(f, "unknown variable '${name}' (known: $F, $T, $DT)")
            }
            ErrorKind::UnknownIdentifier(name) => write!(
                f,
                "unknown identifier '{name}' (use ch(\"{name}\") to read a parameter)"
            ),
            ErrorKind::UnknownComponent { component, ty } => {
                write!(f, "{ty} has no component '.{component}'")
            }
            ErrorKind::Arity {
                function,
                expected,
                found,
            } => {
                let plural = if *expected == "1" { "" } else { "s" };
                write!(
                    f,
                    "'{function}' takes {expected} argument{plural}, found {found}"
                )
            }
            ErrorKind::TypeMismatch {
                context,
                expected,
                found,
            } => write!(
                f,
                "type mismatch in {context}: expected {expected}, found {found}"
            ),
            ErrorKind::UnknownChannel(path) => write!(f, "unknown parameter path \"{path}\""),
            ErrorKind::ChannelPathNotLiteral => {
                f.write_str("ch() takes exactly one string literal path")
            }
            ErrorKind::UnsupportedChannelType { path, ty } => write!(
                f,
                "parameter \"{path}\" has type {ty}, which expressions cannot use"
            ),
            ErrorKind::DivisionByZero => f.write_str("integer division by zero"),
            ErrorKind::IntegerOverflow => f.write_str("integer overflow"),
            ErrorKind::MissingSlot { slot } => {
                write!(f, "no value supplied for parameter slot {slot}")
            }
            ErrorKind::SlotTypeMismatch {
                slot,
                expected,
                found,
            } => write!(
                f,
                "parameter slot {slot} holds {found}, expected {expected}"
            ),
            ErrorKind::Internal(what) => write!(f, "internal expression error: {what}"),
        }
    }
}

/// An error with the byte span of the source it refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExprError {
    pub kind: ErrorKind,
    pub span: Span,
}

impl ExprError {
    /// An error of `kind` at `span`.
    #[must_use]
    pub fn new(kind: ErrorKind, span: Span) -> Self {
        ExprError { kind, span }
    }
}

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at {}", self.kind, self.span)
    }
}

impl std::error::Error for ExprError {}
