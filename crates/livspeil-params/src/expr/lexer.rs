// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Tokenizer. All syntax is ASCII; non-ASCII characters may only appear
//! inside string literals.

use super::error::{ErrorKind, ExprError, Span};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Tok {
    Int(i64),
    Float(f64),
    Str(String),
    Ident(String),
    /// `$NAME`, without the dollar sign.
    Var(String),
    LParen,
    RParen,
    Comma,
    Dot,
    Question,
    Colon,
    Plus,
    Minus,
    Star,
    StarStar,
    Slash,
    Percent,
    Bang,
    AndAnd,
    OrOr,
    EqEq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    Eof,
}

impl Tok {
    /// Human-readable description for "expected X, found Y" messages.
    pub(crate) fn describe(&self) -> String {
        let sym = match self {
            Tok::Int(i) => return format!("number {i}"),
            Tok::Float(v) => return format!("number {v:?}"),
            Tok::Str(s) => return format!("string {s:?}"),
            Tok::Ident(s) => return format!("identifier '{s}'"),
            Tok::Var(s) => return format!("variable '${s}'"),
            Tok::Eof => return "end of input".to_owned(),
            Tok::LParen => "(",
            Tok::RParen => ")",
            Tok::Comma => ",",
            Tok::Dot => ".",
            Tok::Question => "?",
            Tok::Colon => ":",
            Tok::Plus => "+",
            Tok::Minus => "-",
            Tok::Star => "*",
            Tok::StarStar => "**",
            Tok::Slash => "/",
            Tok::Percent => "%",
            Tok::Bang => "!",
            Tok::AndAnd => "&&",
            Tok::OrOr => "||",
            Tok::EqEq => "==",
            Tok::NotEq => "!=",
            Tok::Lt => "<",
            Tok::Le => "<=",
            Tok::Gt => ">",
            Tok::Ge => ">=",
        };
        format!("'{sym}'")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Token {
    pub tok: Tok,
    pub span: Span,
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Split `src` into tokens, always ending with [`Tok::Eof`].
pub(crate) fn lex(src: &str) -> Result<Vec<Token>, ExprError> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        let start = i;
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let peek = bytes.get(i + 1).copied();
        let (tok, len) = match b {
            b'(' => (Tok::LParen, 1),
            b')' => (Tok::RParen, 1),
            b',' => (Tok::Comma, 1),
            b'.' => (Tok::Dot, 1),
            b'?' => (Tok::Question, 1),
            b':' => (Tok::Colon, 1),
            b'+' => (Tok::Plus, 1),
            b'-' => (Tok::Minus, 1),
            b'*' if peek == Some(b'*') => (Tok::StarStar, 2),
            b'*' => (Tok::Star, 1),
            b'/' => (Tok::Slash, 1),
            b'%' => (Tok::Percent, 1),
            b'!' if peek == Some(b'=') => (Tok::NotEq, 2),
            b'!' => (Tok::Bang, 1),
            b'&' if peek == Some(b'&') => (Tok::AndAnd, 2),
            b'|' if peek == Some(b'|') => (Tok::OrOr, 2),
            b'=' if peek == Some(b'=') => (Tok::EqEq, 2),
            b'=' => {
                return Err(ExprError::new(
                    ErrorKind::Expected {
                        expected: "an operator ('==' compares)",
                        found: "'='".to_owned(),
                    },
                    Span::new(i, i + 1),
                ));
            }
            b'<' if peek == Some(b'=') => (Tok::Le, 2),
            b'<' => (Tok::Lt, 1),
            b'>' if peek == Some(b'=') => (Tok::Ge, 2),
            b'>' => (Tok::Gt, 1),
            b'"' => {
                let (s, len) = lex_string(src, i)?;
                (Tok::Str(s), len)
            }
            b'$' => {
                let mut j = i + 1;
                if !bytes.get(j).copied().is_some_and(is_ident_start) {
                    return Err(ExprError::new(
                        ErrorKind::UnexpectedChar('$'),
                        Span::new(i, i + 1),
                    ));
                }
                while bytes.get(j).copied().is_some_and(is_ident_continue) {
                    j += 1;
                }
                (Tok::Var(src[i + 1..j].to_owned()), j - i)
            }
            b if b.is_ascii_digit() => lex_number(src, i)?,
            b if is_ident_start(b) => {
                let mut j = i + 1;
                while bytes.get(j).copied().is_some_and(is_ident_continue) {
                    j += 1;
                }
                (Tok::Ident(src[i..j].to_owned()), j - i)
            }
            _ => {
                let c = src[i..].chars().next().unwrap_or('\u{fffd}');
                return Err(ExprError::new(
                    ErrorKind::UnexpectedChar(c),
                    Span::new(i, i + c.len_utf8()),
                ));
            }
        };
        i += len;
        out.push(Token {
            tok,
            span: Span::new(start, i),
        });
    }
    out.push(Token {
        tok: Tok::Eof,
        span: Span::new(src.len(), src.len()),
    });
    Ok(out)
}

/// Number: `digits ('.' digits)? ([eE] [+-]? digits)?`. Without a fraction or
/// exponent it is an `Int`.
fn lex_number(src: &str, start: usize) -> Result<(Tok, usize), ExprError> {
    let bytes = src.as_bytes();
    let digit_at = |j: usize| bytes.get(j).is_some_and(u8::is_ascii_digit);
    let mut j = start;
    while digit_at(j) {
        j += 1;
    }
    let mut float = false;
    if bytes.get(j) == Some(&b'.') && digit_at(j + 1) {
        float = true;
        j += 1;
        while digit_at(j) {
            j += 1;
        }
    }
    if matches!(bytes.get(j), Some(b'e' | b'E')) {
        let k = if matches!(bytes.get(j + 1), Some(b'+' | b'-')) {
            j + 2
        } else {
            j + 1
        };
        if digit_at(k) {
            float = true;
            j = k;
            while digit_at(j) {
                j += 1;
            }
        }
    }
    let text = &src[start..j];
    let span = Span::new(start, j);
    if float {
        match text.parse::<f64>() {
            Ok(v) if v.is_finite() => Ok((Tok::Float(v), j - start)),
            _ => Err(ExprError::new(
                ErrorKind::InvalidNumber("float literal out of range"),
                span,
            )),
        }
    } else {
        match text.parse::<i64>() {
            Ok(v) => Ok((Tok::Int(v), j - start)),
            Err(_) => Err(ExprError::new(
                ErrorKind::InvalidNumber("integer literal out of range"),
                span,
            )),
        }
    }
}

/// String literal starting at the opening quote. Escapes: `\" \\ \n \t`.
fn lex_string(src: &str, start: usize) -> Result<(String, usize), ExprError> {
    let mut out = String::new();
    let mut chars = src[start + 1..].char_indices();
    while let Some((off, c)) = chars.next() {
        let pos = start + 1 + off;
        match c {
            '"' => return Ok((out, pos + 1 - start)),
            '\\' => match chars.next() {
                Some((_, '"')) => out.push('"'),
                Some((_, '\\')) => out.push('\\'),
                Some((_, 'n')) => out.push('\n'),
                Some((_, 't')) => out.push('\t'),
                Some((_, other)) => {
                    return Err(ExprError::new(
                        ErrorKind::InvalidEscape(other),
                        Span::new(pos, pos + 1 + other.len_utf8()),
                    ));
                }
                None => break,
            },
            c => out.push(c),
        }
    }
    Err(ExprError::new(
        ErrorKind::UnterminatedString,
        Span::new(start, src.len()),
    ))
}
