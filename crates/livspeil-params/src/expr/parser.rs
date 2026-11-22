// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Pratt parser producing an untyped syntax tree.
//!
//! Both the parser's recursion and the depth of the resulting tree are
//! bounded by [`MAX_DEPTH`], so every later recursive pass (type checking,
//! code generation, `Drop`) is bounded as well.

use super::MAX_DEPTH;
use super::error::{ErrorKind, ExprError, Span};
use super::lexer::{Tok, Token, lex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
}

impl BinOp {
    pub(crate) fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Mod => "%",
            BinOp::Pow => "**",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::And => "&&",
            BinOp::Or => "||",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ExprKind {
    Int(i64),
    Float(f64),
    Str(String),
    /// `$NAME`.
    Var(String),
    /// Bare identifier: `true`, `false`, `pi`, `tau`.
    Ident(String),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// `c ? a : b`.
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Call {
        name: String,
        name_span: Span,
        args: Vec<Expr>,
    },
    /// `base.name`.
    Member {
        base: Box<Expr>,
        name: String,
        name_span: Span,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Expr {
    pub kind: ExprKind,
    pub span: Span,
    /// Height of this subtree (leaves are 1).
    pub depth: usize,
}

/// Nodes and errors are boxed inside the parser so that every `Result` on
/// the recursive path is pointer-sized.
type PResult = Result<Box<Expr>, Box<ExprError>>;

fn error(kind: ErrorKind, span: Span) -> Box<ExprError> {
    Box::new(ExprError::new(kind, span))
}

impl Expr {
    fn new(kind: ExprKind, span: Span) -> PResult {
        let child = match &kind {
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Str(_)
            | ExprKind::Var(_)
            | ExprKind::Ident(_) => 0,
            ExprKind::Unary(_, a) => a.depth,
            ExprKind::Binary(_, a, b) => a.depth.max(b.depth),
            ExprKind::Cond(a, b, c) => a.depth.max(b.depth).max(c.depth),
            ExprKind::Call { args, .. } => args.iter().map(|a| a.depth).max().unwrap_or(0),
            ExprKind::Member { base, .. } => base.depth,
        };
        let depth = child + 1;
        if depth > MAX_DEPTH {
            return Err(error(ErrorKind::TooDeep { limit: MAX_DEPTH }, span));
        }
        Ok(Box::new(Expr { kind, span, depth }))
    }
}

/// Binding powers. Higher binds tighter. `(left, right)`: left-associative
/// operators have `right = left + 1`, right-associative ones `right = left - 1`.
fn infix_bp(tok: &Tok) -> Option<(u8, u8, Option<BinOp>)> {
    Some(match tok {
        Tok::Question => (2, 1, None),
        Tok::OrOr => (3, 4, Some(BinOp::Or)),
        Tok::AndAnd => (5, 6, Some(BinOp::And)),
        Tok::EqEq => (7, 8, Some(BinOp::Eq)),
        Tok::NotEq => (7, 8, Some(BinOp::Ne)),
        Tok::Lt => (9, 10, Some(BinOp::Lt)),
        Tok::Le => (9, 10, Some(BinOp::Le)),
        Tok::Gt => (9, 10, Some(BinOp::Gt)),
        Tok::Ge => (9, 10, Some(BinOp::Ge)),
        Tok::Plus => (11, 12, Some(BinOp::Add)),
        Tok::Minus => (11, 12, Some(BinOp::Sub)),
        Tok::Star => (13, 14, Some(BinOp::Mul)),
        Tok::Slash => (13, 14, Some(BinOp::Div)),
        Tok::Percent => (13, 14, Some(BinOp::Mod)),
        Tok::StarStar => (18, 17, Some(BinOp::Pow)),
        _ => return None,
    })
}

/// Right binding power of prefix `-` and `!`: tighter than `*`, looser than
/// `**`, so `-2 ** 2 == -(2 ** 2)`.
const PREFIX_BP: u8 = 15;
/// Binding power of postfix `.component`.
const POSTFIX_BP: u8 = 20;

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    depth: usize,
}

/// Parse a whole expression.
pub(crate) fn parse(src: &str) -> PResult {
    let tokens = lex(src)?;
    let mut p = Parser {
        tokens,
        pos: 0,
        depth: 0,
    };
    let expr = p.expr(0)?;
    let tok = p.peek();
    if tok.tok != Tok::Eof {
        return Err(p.expected("an operator or end of input"));
    }
    Ok(expr)
}

impl Parser {
    fn peek(&self) -> &Token {
        // The token list always ends with Eof and `pos` never passes it.
        &self.tokens[self.pos.min(self.tokens.len() - 1)]
    }

    /// Consume the next token (never past Eof) and return its span.
    fn bump(&mut self) -> Span {
        let span = self.peek().span;
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        span
    }

    #[inline(never)]
    fn expected(&self, expected: &'static str) -> Box<ExprError> {
        let t = self.peek();
        error(
            ErrorKind::Expected {
                expected,
                found: t.tok.describe(),
            },
            t.span,
        )
    }

    fn expect(&mut self, tok: &Tok, expected: &'static str) -> Result<Span, Box<ExprError>> {
        if &self.peek().tok == tok {
            Ok(self.bump())
        } else {
            Err(self.expected(expected))
        }
    }

    // The functions on the recursive path (`expr`, `expr_inner`, `prefix`,
    // `args`) are kept lean; node construction and error paths live in
    // separate non-recursive helpers. Unoptimized builds give every
    // temporary its own stack slot, so this keeps the stack used per
    // nesting level small even in debug builds.

    fn expr(&mut self, min_bp: u8) -> PResult {
        self.depth += 1;
        let result = if self.depth > MAX_DEPTH {
            Err(self.too_deep())
        } else {
            self.expr_inner(min_bp)
        };
        self.depth -= 1;
        result
    }

    fn expr_inner(&mut self, min_bp: u8) -> PResult {
        let mut lhs = self.prefix()?;
        loop {
            lhs = match self.infix(min_bp) {
                Infix::Stop => return Ok(lhs),
                Infix::Member => self.member(lhs)?,
                Infix::Cond(r_bp) => {
                    let then = self.expr(0)?;
                    self.expect(&Tok::Colon, "':' in conditional")?;
                    let otherwise = self.expr(r_bp)?;
                    cond(lhs, then, otherwise)?
                }
                Infix::Binary(op, r_bp) => {
                    let rhs = self.expr(r_bp)?;
                    binary(op, lhs, rhs)?
                }
            };
        }
    }

    fn prefix(&mut self) -> PResult {
        match self.prefix_kind() {
            Prefix::Paren(open) => {
                let inner = self.expr(0)?;
                self.close_paren(open, inner)
            }
            Prefix::Unary(op, span) => {
                let operand = self.expr(PREFIX_BP)?;
                unary(op, span, operand)
            }
            Prefix::Call(name, span) => {
                let args = self.args()?;
                self.finish_call(name, span, args)
            }
            Prefix::Leaf => self.leaf(),
        }
    }

    /// Comma-separated arguments up to (not including) `)`.
    fn args(&mut self) -> Result<Vec<Expr>, Box<ExprError>> {
        let mut args = Vec::new();
        if self.peek().tok == Tok::RParen {
            return Ok(args);
        }
        loop {
            args.push(*self.expr(0)?);
            if self.peek().tok != Tok::Comma {
                return Ok(args);
            }
            self.bump();
        }
    }

    // ---- non-recursive helpers ----

    #[inline(never)]
    fn too_deep(&self) -> Box<ExprError> {
        error(ErrorKind::TooDeep { limit: MAX_DEPTH }, self.peek().span)
    }

    /// Classify the next token as an infix/postfix operator binding at least
    /// as tightly as `min_bp`, consuming it if so.
    #[inline(never)]
    fn infix(&mut self, min_bp: u8) -> Infix {
        let tok = &self.peek().tok;
        let result = if *tok == Tok::Dot {
            if POSTFIX_BP < min_bp {
                return Infix::Stop;
            }
            Infix::Member
        } else {
            match infix_bp(tok) {
                Some((l_bp, _, _)) if l_bp < min_bp => return Infix::Stop,
                Some((_, r_bp, None)) => Infix::Cond(r_bp),
                Some((_, r_bp, Some(op))) => Infix::Binary(op, r_bp),
                None => return Infix::Stop,
            }
        };
        self.bump();
        result
    }

    /// Parse `.name` after `base` (the dot is already consumed).
    #[inline(never)]
    fn member(&mut self, base: Box<Expr>) -> PResult {
        let Tok::Ident(name) = &self.peek().tok else {
            return Err(self.expected("a component name after '.'"));
        };
        let name = name.clone();
        let name_span = self.bump();
        let span = base.span.to(name_span);
        Expr::new(
            ExprKind::Member {
                base,
                name,
                name_span,
            },
            span,
        )
    }

    /// Classify (and for non-leaves, consume) the start of a prefix
    /// expression.
    #[inline(never)]
    fn prefix_kind(&mut self) -> Prefix {
        let t = self.peek();
        let span = t.span;
        let kind = match &t.tok {
            Tok::LParen => Prefix::Paren(span),
            Tok::Minus => Prefix::Unary(UnOp::Neg, span),
            Tok::Bang => Prefix::Unary(UnOp::Not, span),
            Tok::Ident(name) if self.peek_at(1) == &Tok::LParen => {
                let name = name.clone();
                self.bump();
                Prefix::Call(name, span)
            }
            _ => return Prefix::Leaf,
        };
        self.bump();
        kind
    }

    fn peek_at(&self, offset: usize) -> &Tok {
        let i = (self.pos + offset).min(self.tokens.len() - 1);
        &self.tokens[i].tok
    }

    #[inline(never)]
    fn close_paren(&mut self, open: Span, mut inner: Box<Expr>) -> PResult {
        let close = self.expect(&Tok::RParen, "')'")?;
        // Include the parentheses so errors point at what the user wrote.
        inner.span = open.to(close);
        Ok(inner)
    }

    #[inline(never)]
    fn finish_call(&mut self, name: String, name_span: Span, args: Vec<Expr>) -> PResult {
        let close = self.expect(&Tok::RParen, "',' or ')'")?;
        Expr::new(
            ExprKind::Call {
                name,
                name_span,
                args,
            },
            name_span.to(close),
        )
    }

    #[inline(never)]
    fn leaf(&mut self) -> PResult {
        let t = self.peek();
        let span = t.span;
        let kind = match &t.tok {
            Tok::Int(v) => ExprKind::Int(*v),
            Tok::Float(v) => ExprKind::Float(*v),
            Tok::Str(s) => ExprKind::Str(s.clone()),
            Tok::Var(name) => ExprKind::Var(name.clone()),
            Tok::Ident(name) => ExprKind::Ident(name.clone()),
            _ => return Err(self.expected("an expression")),
        };
        self.bump();
        Expr::new(kind, span)
    }
}

enum Infix {
    Stop,
    Member,
    Cond(u8),
    Binary(BinOp, u8),
}

enum Prefix {
    Paren(Span),
    Unary(UnOp, Span),
    Call(String, Span),
    Leaf,
}

#[inline(never)]
fn cond(c: Box<Expr>, a: Box<Expr>, b: Box<Expr>) -> PResult {
    let span = c.span.to(b.span);
    Expr::new(ExprKind::Cond(c, a, b), span)
}

#[inline(never)]
fn binary(op: BinOp, a: Box<Expr>, b: Box<Expr>) -> PResult {
    let span = a.span.to(b.span);
    Expr::new(ExprKind::Binary(op, a, b), span)
}

#[inline(never)]
fn unary(op: UnOp, op_span: Span, operand: Box<Expr>) -> PResult {
    let span = op_span.to(operand.span);
    Expr::new(ExprKind::Unary(op, operand), span)
}
