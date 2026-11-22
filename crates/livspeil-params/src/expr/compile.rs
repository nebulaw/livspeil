// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Type checking and code generation in a single pass over the syntax tree.

use livspeil_node_api::PortType;

use super::error::{ErrorKind, ExprError, Span};
use super::parser::{BinOp, Expr, ExprKind, UnOp};
use super::vm::{Cmp, F1, F2, F3, I2, Op};
use super::{Env, Program};

type Ty = PortType;

/// Errors are boxed inside the compiler so that every `Result` on the
/// recursive path is pointer-sized.
fn error(kind: ErrorKind, span: Span) -> Box<ExprError> {
    Box::new(ExprError::new(kind, span))
}

fn is_num(t: Ty) -> bool {
    matches!(t, Ty::Int | Ty::Float)
}

fn vec_len(t: Ty) -> Option<u8> {
    match t {
        Ty::Vec2 => Some(2),
        Ty::Vec3 => Some(3),
        Ty::Vec4 => Some(4),
        _ => None,
    }
}

fn vec_ty(n: u8) -> Ty {
    match n {
        2 => Ty::Vec2,
        3 => Ty::Vec3,
        _ => Ty::Vec4,
    }
}

/// Types an expression may produce.
fn is_expr_type(t: Ty) -> bool {
    matches!(
        t,
        Ty::Float | Ty::Int | Ty::Bool | Ty::Vec2 | Ty::Vec3 | Ty::Vec4 | Ty::Color | Ty::String
    )
}

/// Built-in functions. `if` and `ch` are special forms.
#[derive(Debug, Clone, Copy)]
enum Fun {
    Math(F1),
    Abs,
    Sign,
    Float2(F2),
    MinMax(I2, F2),
    Mod,
    Clamp,
    Lerp,
    Smoothstep,
    Length,
    Dot,
    Cross,
    Normalize,
    Vec(u8),
    Oklch,
    Len,
    Upper,
    Lower,
    Str,
    Concat,
    ToInt,
    ToFloat,
    If,
    Ch,
}

/// `(function, min arity, max arity)`.
fn lookup(name: &str) -> Option<(Fun, usize, usize)> {
    use Fun::*;
    let one = |f| Some((f, 1, 1));
    let two = |f| Some((f, 2, 2));
    let three = |f| Some((f, 3, 3));
    match name {
        "sin" => one(Math(F1::Sin)),
        "cos" => one(Math(F1::Cos)),
        "tan" => one(Math(F1::Tan)),
        "asin" => one(Math(F1::Asin)),
        "acos" => one(Math(F1::Acos)),
        "atan" => one(Math(F1::Atan)),
        "sqrt" => one(Math(F1::Sqrt)),
        "exp" => one(Math(F1::Exp)),
        "ln" => one(Math(F1::Ln)),
        "log10" => one(Math(F1::Log10)),
        "floor" => one(Math(F1::Floor)),
        "ceil" => one(Math(F1::Ceil)),
        "round" => one(Math(F1::Round)),
        "fract" => one(Math(F1::Fract)),
        "abs" => one(Abs),
        "sign" => one(Sign),
        "atan2" => two(Float2(F2::Atan2)),
        "pow" => two(Float2(F2::Pow)),
        "step" => two(Float2(F2::Step)),
        "min" => two(MinMax(I2::Min, F2::Min)),
        "max" => two(MinMax(I2::Max, F2::Max)),
        "mod" => two(Mod),
        "clamp" => three(Clamp),
        "lerp" => three(Lerp),
        "smoothstep" => three(Smoothstep),
        "length" => one(Length),
        "dot" => two(Dot),
        "cross" => two(Cross),
        "normalize" => one(Normalize),
        "vec2" => two(Vec(2)),
        "vec3" => three(Vec(3)),
        "vec4" => Some((Vec(4), 4, 4)),
        "oklch" => Some((Oklch, 3, 4)),
        "len" => one(Len),
        "upper" => one(Upper),
        "lower" => one(Lower),
        "str" => one(Str),
        "concat" => Some((Concat, 1, usize::MAX)),
        "int" => one(ToInt),
        "float" => one(ToFloat),
        "if" => three(If),
        "ch" => one(Ch),
        _ => None,
    }
}

fn arity_text(min: usize, max: usize) -> &'static str {
    match (min, max) {
        (1, 1) => "1",
        (2, 2) => "2",
        (3, 3) => "3",
        (4, 4) => "4",
        (3, 4) => "3 or 4",
        _ => "at least 1",
    }
}

fn mismatch(
    context: impl Into<String>,
    expected: impl Into<String>,
    found: Ty,
    span: Span,
) -> Box<ExprError> {
    error(
        ErrorKind::TypeMismatch {
            context: context.into(),
            expected: expected.into(),
            found,
        },
        span,
    )
}

pub(crate) struct Gen<'e> {
    env: &'e dyn Env,
    code: Vec<Op>,
    spans: Vec<Span>,
    strings: Vec<String>,
    deps: Vec<usize>,
    time: bool,
    num: isize,
    max_num: isize,
    strs: isize,
}

impl<'e> Gen<'e> {
    pub(crate) fn new(env: &'e dyn Env) -> Self {
        Gen {
            env,
            code: Vec::new(),
            spans: Vec::new(),
            strings: Vec::new(),
            deps: Vec::new(),
            time: false,
            num: 0,
            max_num: 0,
            strs: 0,
        }
    }

    pub(crate) fn finish(mut self, root: &Expr) -> Result<Program, ExprError> {
        let ty = self.gen_expr(root).map_err(|e| *e)?;
        let expected_depth = if ty == Ty::String { (0, 1) } else { (1, 0) };
        if (self.num, self.strs) != expected_depth {
            return Err(ExprError::new(
                ErrorKind::Internal("unbalanced stack after compilation"),
                root.span,
            ));
        }
        self.deps.sort_unstable();
        self.deps.dedup();
        Ok(Program {
            code: self.code,
            spans: self.spans,
            strings: self.strings,
            ty,
            deps: self.deps,
            time_dependent: self.time,
            max_num: usize::try_from(self.max_num).unwrap_or(0),
        })
    }

    fn emit(&mut self, op: Op, span: Span) -> usize {
        let (dn, ds) = op.effect();
        self.num += dn;
        self.strs += ds;
        self.max_num = self.max_num.max(self.num);
        self.code.push(op);
        self.spans.push(span);
        self.code.len() - 1
    }

    fn patch(&mut self, at: usize, op: Op) {
        if let Some(slot) = self.code.get_mut(at) {
            *slot = op;
        }
    }

    fn here(&self) -> usize {
        self.code.len()
    }

    /// Widen an Int that sits `depth` numeric cells below the top.
    fn widen(&mut self, ty: Ty, depth: u32, span: Span) {
        if ty == Ty::Int {
            self.emit(Op::IntToFloat(depth), span);
        }
    }

    // Functions that recurse (`gen_expr` and the `gen_*` it calls) only
    // generate children and then hand over to non-recursive, never-inlined
    // helpers for type dispatch, so the stack used per nesting level stays
    // small even in unoptimized builds.

    fn gen_expr(&mut self, e: &Expr) -> Result<Ty, Box<ExprError>> {
        let span = e.span;
        match &e.kind {
            ExprKind::Unary(op, a) => self.gen_unary(*op, a, span),
            ExprKind::Binary(op, a, b) => self.gen_binary(*op, a, b, span),
            ExprKind::Cond(c, a, b) => self.gen_cond(c, a, b, span),
            ExprKind::Call {
                name,
                name_span,
                args,
            } => self.gen_call(name, *name_span, args, span),
            ExprKind::Member {
                base,
                name,
                name_span,
            } => self.gen_member(base, name, *name_span),
            _ => self.gen_leaf(e),
        }
    }

    #[inline(never)]
    fn gen_leaf(&mut self, e: &Expr) -> Result<Ty, Box<ExprError>> {
        let span = e.span;
        match &e.kind {
            ExprKind::Int(i) => {
                self.emit(Op::PushI(*i), span);
                Ok(Ty::Int)
            }
            ExprKind::Float(x) => {
                self.emit(Op::PushF(*x), span);
                Ok(Ty::Float)
            }
            ExprKind::Str(s) => {
                self.strings.push(s.clone());
                self.emit(Op::PushS(self.strings.len() - 1), span);
                Ok(Ty::String)
            }
            ExprKind::Var(name) => {
                self.time = true;
                let (op, ty) = match name.as_str() {
                    "F" => (Op::Frame, Ty::Int),
                    "T" => (Op::Time, Ty::Float),
                    "DT" => (Op::Dt, Ty::Float),
                    _ => {
                        return Err(error(ErrorKind::UnknownVariable(name.clone()), span));
                    }
                };
                self.emit(op, span);
                Ok(ty)
            }
            ExprKind::Ident(name) => {
                let op = match name.as_str() {
                    "true" => Op::PushB(true),
                    "false" => Op::PushB(false),
                    "pi" => Op::PushF(std::f64::consts::PI),
                    "tau" => Op::PushF(std::f64::consts::TAU),
                    _ => {
                        return Err(error(ErrorKind::UnknownIdentifier(name.clone()), span));
                    }
                };
                self.emit(op, span);
                Ok(if matches!(op, Op::PushB(_)) {
                    Ty::Bool
                } else {
                    Ty::Float
                })
            }
            _ => Err(error(ErrorKind::Internal("not a leaf"), span)),
        }
    }

    fn gen_unary(&mut self, op: UnOp, a: &Expr, span: Span) -> Result<Ty, Box<ExprError>> {
        let t = self.gen_expr(a)?;
        self.unary_typed(op, t, a, span)
    }

    #[inline(never)]
    fn unary_typed(&mut self, op: UnOp, t: Ty, a: &Expr, span: Span) -> Result<Ty, Box<ExprError>> {
        match op {
            UnOp::Neg => match t {
                Ty::Int => {
                    self.emit(Op::NegI, span);
                    Ok(t)
                }
                Ty::Float => {
                    self.emit(Op::F1(F1::Neg), span);
                    Ok(t)
                }
                _ => match vec_len(t) {
                    Some(n) => {
                        self.emit(Op::V1(F1::Neg, n), span);
                        Ok(t)
                    }
                    None => Err(mismatch(
                        "operand of '-'",
                        "Int, Float or vector",
                        t,
                        a.span,
                    )),
                },
            },
            UnOp::Not => {
                if t != Ty::Bool {
                    return Err(mismatch("operand of '!'", "Bool", t, a.span));
                }
                self.emit(Op::Not, span);
                Ok(Ty::Bool)
            }
        }
    }

    fn gen_bool(&mut self, e: &Expr, context: &'static str) -> Result<(), Box<ExprError>> {
        let t = self.gen_expr(e)?;
        check_bool(t, e.span, context)
    }

    fn gen_binary(
        &mut self,
        op: BinOp,
        a: &Expr,
        b: &Expr,
        span: Span,
    ) -> Result<Ty, Box<ExprError>> {
        if matches!(op, BinOp::And | BinOp::Or) {
            let ctx = if op == BinOp::And {
                "operator '&&'"
            } else {
                "operator '||'"
            };
            // a && b  =>  a; jif L1; b; jmp L2; L1: push false; L2:
            // a || b  =>  a; jif L1; push true; jmp L2; L1: b; L2:
            self.gen_bool(a, ctx)?;
            let jif = self.emit(Op::JumpIfFalse(0), span);
            let base = self.num;
            if op == BinOp::And {
                self.gen_bool(b, ctx)?;
            } else {
                self.emit(Op::PushB(true), span);
            }
            let jmp = self.emit(Op::Jump(0), span);
            self.num = base;
            let l1 = self.here();
            self.patch(jif, Op::JumpIfFalse(l1));
            if op == BinOp::And {
                self.emit(Op::PushB(false), span);
            } else {
                self.gen_bool(b, ctx)?;
            }
            let l2 = self.here();
            self.patch(jmp, Op::Jump(l2));
            return Ok(Ty::Bool);
        }

        let lt = self.gen_expr(a)?;
        let rt = self.gen_expr(b)?;
        self.binary_typed(op, lt, rt, a.span, b.span, span)
    }

    #[inline(never)]
    fn binary_typed(
        &mut self,
        op: BinOp,
        lt: Ty,
        rt: Ty,
        a_span: Span,
        b_span: Span,
        span: Span,
    ) -> Result<Ty, Box<ExprError>> {
        let ctx = format!("operator '{}'", op.symbol());
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
                self.gen_arith(op, lt, rt, a_span, b_span, span, &ctx)
            }
            BinOp::Pow => {
                if !is_num(lt) {
                    return Err(mismatch(ctx, "Int or Float", lt, a_span));
                }
                if !is_num(rt) {
                    return Err(mismatch(ctx, "Int or Float", rt, b_span));
                }
                self.widen(lt, 1, span);
                self.widen(rt, 0, span);
                self.emit(Op::F2(F2::Pow), span);
                Ok(Ty::Float)
            }
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::Eq | BinOp::Ne => {
                let cmp = match op {
                    BinOp::Lt => Cmp::Lt,
                    BinOp::Le => Cmp::Le,
                    BinOp::Gt => Cmp::Gt,
                    BinOp::Ge => Cmp::Ge,
                    BinOp::Eq => Cmp::Eq,
                    _ => Cmp::Ne,
                };
                let ordering = !matches!(op, BinOp::Eq | BinOp::Ne);
                let neg = op == BinOp::Ne;
                if is_num(lt) && is_num(rt) {
                    if lt == Ty::Int && rt == Ty::Int {
                        self.emit(Op::CmpI(cmp), span);
                    } else {
                        self.widen(lt, 1, span);
                        self.widen(rt, 0, span);
                        self.emit(Op::CmpF(cmp), span);
                    }
                    return Ok(Ty::Bool);
                }
                if ordering {
                    if !is_num(lt) {
                        return Err(mismatch(ctx, "Int or Float", lt, a_span));
                    }
                    return Err(mismatch(ctx, "Int or Float", rt, b_span));
                }
                if lt != rt {
                    let expected = if is_num(lt) {
                        "Int or Float".to_owned()
                    } else {
                        lt.name().to_owned()
                    };
                    return Err(mismatch(ctx, expected, rt, b_span));
                }
                match lt {
                    Ty::Bool => self.emit(Op::EqB(neg), span),
                    Ty::String => self.emit(Op::EqS(neg), span),
                    Ty::Color => self.emit(Op::EqV(4, neg), span),
                    _ => {
                        let n = vec_len(lt).ok_or_else(|| {
                            mismatch(ctx.clone(), "a comparable type", lt, a_span)
                        })?;
                        self.emit(Op::EqV(n, neg), span)
                    }
                };
                Ok(Ty::Bool)
            }
            BinOp::And | BinOp::Or => Err(error(
                ErrorKind::Internal("logical operator reached arithmetic path"),
                span,
            )),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn gen_arith(
        &mut self,
        op: BinOp,
        lt: Ty,
        rt: Ty,
        ls: Span,
        rs: Span,
        span: Span,
        ctx: &str,
    ) -> Result<Ty, Box<ExprError>> {
        let (i2, f2) = match op {
            BinOp::Add => (I2::Add, F2::Add),
            BinOp::Sub => (I2::Sub, F2::Sub),
            BinOp::Mul => (I2::Mul, F2::Mul),
            BinOp::Div => (I2::Div, F2::Div),
            _ => (I2::Mod, F2::Mod),
        };
        if lt == Ty::Int && rt == Ty::Int {
            self.emit(Op::I2(i2), span);
            return Ok(Ty::Int);
        }
        if is_num(lt) && is_num(rt) {
            self.widen(lt, 1, span);
            self.widen(rt, 0, span);
            self.emit(Op::F2(f2), span);
            return Ok(Ty::Float);
        }
        if op == BinOp::Add && lt == Ty::String {
            if rt != Ty::String {
                return Err(mismatch(ctx, "String (use str() to convert)", rt, rs));
            }
            self.emit(Op::Concat, span);
            return Ok(Ty::String);
        }
        let scalar_ops = matches!(op, BinOp::Mul | BinOp::Div);
        if let Some(n) = vec_len(lt) {
            if rt == lt && op != BinOp::Mod {
                self.emit(Op::V2(f2, n), span);
                return Ok(lt);
            }
            if scalar_ops && is_num(rt) {
                self.widen(rt, 0, span);
                self.emit(Op::VS(f2, n), span);
                return Ok(lt);
            }
            if op == BinOp::Mod {
                return Err(mismatch(ctx, "Int or Float", lt, ls));
            }
            let expected = if scalar_ops {
                format!("{lt} or Float")
            } else {
                lt.name().to_owned()
            };
            return Err(mismatch(ctx, expected, rt, rs));
        }
        if is_num(lt) {
            if op == BinOp::Mul
                && let Some(n) = vec_len(rt)
            {
                self.widen(lt, 1, span);
                self.emit(Op::SV(f2, n), span);
                return Ok(rt);
            }
            let expected = if op == BinOp::Mul {
                "Int, Float or vector"
            } else {
                "Int or Float"
            };
            return Err(mismatch(ctx, expected, rt, rs));
        }
        let expected = match op {
            BinOp::Add => "Int, Float, vector or String",
            BinOp::Mod => "Int or Float",
            _ => "Int, Float or vector",
        };
        Err(mismatch(ctx, expected, lt, ls))
    }

    fn gen_cond(&mut self, c: &Expr, a: &Expr, b: &Expr, span: Span) -> Result<Ty, Box<ExprError>> {
        // c; jif ELSE; a; [widen]; jmp END; ELSE: b; [widen]; END:
        self.gen_bool(c, "condition")?;
        let jif = self.emit(Op::JumpIfFalse(0), span);
        let base = (self.num, self.strs);
        let at = self.gen_expr(a)?;
        let widen_a = self.emit(Op::Nop, a.span);
        let jmp = self.emit(Op::Jump(0), span);
        (self.num, self.strs) = base;
        let else_at = self.here();
        self.patch(jif, Op::JumpIfFalse(else_at));
        let bt = self.gen_expr(b)?;
        let ty = self.unify_branches(at, bt, widen_a, b.span)?;
        let end = self.here();
        self.patch(jmp, Op::Jump(end));
        Ok(ty)
    }

    /// Result type of a conditional; widens an Int branch next to a Float one.
    #[inline(never)]
    fn unify_branches(
        &mut self,
        at: Ty,
        bt: Ty,
        widen_a: usize,
        b_span: Span,
    ) -> Result<Ty, Box<ExprError>> {
        Ok(if at == bt {
            at
        } else if is_num(at) && is_num(bt) {
            if at == Ty::Int {
                self.patch(widen_a, Op::IntToFloat(0));
            } else {
                self.emit(Op::IntToFloat(0), b_span);
            }
            Ty::Float
        } else {
            let expected = if is_num(at) {
                "Int or Float".to_owned()
            } else {
                format!("{at} (same as the other branch)")
            };
            return Err(mismatch("conditional branches", expected, bt, b_span));
        })
    }

    fn gen_member(
        &mut self,
        base: &Expr,
        name: &str,
        name_span: Span,
    ) -> Result<Ty, Box<ExprError>> {
        let t = self.gen_expr(base)?;
        self.member_typed(t, name, name_span, base.span)
    }

    #[inline(never)]
    fn member_typed(
        &mut self,
        t: Ty,
        name: &str,
        name_span: Span,
        base_span: Span,
    ) -> Result<Ty, Box<ExprError>> {
        let unknown = || {
            error(
                ErrorKind::UnknownComponent {
                    component: name.to_owned(),
                    ty: t,
                },
                name_span,
            )
        };
        let (letters, n): (&[u8], u8) = match t {
            Ty::Color => (b"lcha", 4),
            _ => match vec_len(t) {
                Some(n) => (b"xyzw", n),
                None => return Err(unknown()),
            },
        };
        let bytes = name.as_bytes();
        let max_len = if t == Ty::Color { 1 } else { 4 };
        if bytes.is_empty() || bytes.len() > max_len {
            return Err(unknown());
        }
        let mut idx = [0u8; 4];
        for (slot, ch) in idx.iter_mut().zip(bytes) {
            let pos = letters
                .iter()
                .position(|l| l == ch)
                .and_then(|p| u8::try_from(p).ok())
                .filter(|p| *p < n)
                .ok_or_else(unknown)?;
            *slot = pos;
        }
        let span = base_span.to(name_span);
        if bytes.len() == 1 {
            self.emit(Op::Comp(idx[0]), span);
            Ok(Ty::Float)
        } else {
            let out = u8::try_from(bytes.len()).unwrap_or(4);
            self.emit(Op::Swizzle(idx, out), span);
            Ok(vec_ty(out))
        }
    }

    fn gen_call(
        &mut self,
        name: &str,
        name_span: Span,
        args: &[Expr],
        span: Span,
    ) -> Result<Ty, Box<ExprError>> {
        let fun = check_call(name, name_span, args.len(), span)?;
        match fun {
            Fun::If => {
                let [c, a, b] = args else {
                    return Err(error(ErrorKind::Internal("if arity"), span));
                };
                self.gen_cond(c, a, b, span)
            }
            Fun::Ch => self.gen_ch(args, span),
            Fun::Concat => {
                for (i, arg) in args.iter().enumerate() {
                    let t = self.gen_expr(arg)?;
                    self.concat_step(t, i, arg.span, span);
                }
                Ok(Ty::String)
            }
            _ => {
                let mut tys = Vec::with_capacity(args.len());
                for a in args {
                    tys.push(self.gen_expr(a)?);
                }
                self.call_typed(fun, name, &tys, args, span)
            }
        }
    }

    #[inline(never)]
    fn concat_step(&mut self, t: Ty, i: usize, arg_span: Span, span: Span) {
        if t != Ty::String {
            self.emit(Op::StrOf(t), arg_span);
        }
        if i > 0 {
            self.emit(Op::Concat, span);
        }
    }

    /// Type dispatch for ordinary functions once all arguments are generated.
    #[inline(never)]
    fn call_typed(
        &mut self,
        fun: Fun,
        name: &str,
        tys: &[Ty],
        args: &[Expr],
        span: Span,
    ) -> Result<Ty, Box<ExprError>> {
        let arg_ctx = |i: usize| format!("argument {} of '{name}'", i + 1);
        let err = |i: usize, expected: &str| mismatch(arg_ctx(i), expected, tys[i], args[i].span);
        // Numeric cells above argument `i` (all arguments here are numeric).
        let depth = |i: usize| u32::try_from(tys.len() - 1 - i).unwrap_or(0);
        let all_int = tys.iter().all(|t| *t == Ty::Int);
        let all_num = tys.iter().all(|t| is_num(*t));
        // Require all arguments to be Int/Float and widen them to Float.
        macro_rules! floats {
            () => {{
                for (i, t) in tys.iter().enumerate() {
                    if !is_num(*t) {
                        return Err(err(i, "Int or Float"));
                    }
                }
                for (i, t) in tys.iter().enumerate() {
                    self.widen(*t, depth(i), span);
                }
            }};
        }

        let Some(&t0) = tys.first() else {
            return Err(error(ErrorKind::Internal("call without arguments"), span));
        };
        match fun {
            Fun::Abs | Fun::Sign if t0 == Ty::Int => {
                let op = if matches!(fun, Fun::Abs) {
                    Op::AbsI
                } else {
                    Op::SignI
                };
                self.emit(op, span);
                Ok(Ty::Int)
            }
            Fun::Math(_) | Fun::Abs | Fun::Sign => {
                let f = match fun {
                    Fun::Math(f) => f,
                    Fun::Abs => F1::Abs,
                    _ => F1::Sign,
                };
                if is_num(t0) {
                    self.widen(t0, 0, span);
                    self.emit(Op::F1(f), span);
                    Ok(Ty::Float)
                } else if let Some(n) = vec_len(t0) {
                    self.emit(Op::V1(f, n), span);
                    Ok(t0)
                } else {
                    Err(err(0, "Int, Float or vector"))
                }
            }
            Fun::Float2(f) => {
                floats!();
                self.emit(Op::F2(f), span);
                Ok(Ty::Float)
            }
            Fun::MinMax(i2, f2) => {
                if all_int {
                    self.emit(Op::I2(i2), span);
                    return Ok(Ty::Int);
                }
                if let Some(n) = vec_len(t0) {
                    if tys[1] != t0 {
                        return Err(err(1, t0.name()));
                    }
                    self.emit(Op::V2(f2, n), span);
                    return Ok(t0);
                }
                floats!();
                self.emit(Op::F2(f2), span);
                Ok(Ty::Float)
            }
            Fun::Mod => {
                if all_int {
                    self.emit(Op::I2(I2::Mod), span);
                    return Ok(Ty::Int);
                }
                floats!();
                self.emit(Op::F2(F2::Mod), span);
                Ok(Ty::Float)
            }
            Fun::Clamp => {
                if all_int {
                    self.emit(Op::ClampI, span);
                    return Ok(Ty::Int);
                }
                if let Some(n) = vec_len(t0) {
                    for (i, t) in tys.iter().enumerate().skip(1) {
                        if !is_num(*t) {
                            return Err(err(i, "Int or Float"));
                        }
                        self.widen(*t, depth(i), span);
                    }
                    self.emit(Op::VClamp(n), span);
                    return Ok(t0);
                }
                floats!();
                self.emit(Op::F3(F3::Clamp), span);
                Ok(Ty::Float)
            }
            Fun::Lerp => {
                if let Some(n) = vec_len(t0) {
                    if tys[1] != t0 {
                        return Err(err(1, t0.name()));
                    }
                    if !is_num(tys[2]) {
                        return Err(err(2, "Int or Float"));
                    }
                    self.widen(tys[2], 0, span);
                    self.emit(Op::VLerp(n), span);
                    return Ok(t0);
                }
                if !all_num {
                    let i = tys.iter().position(|t| !is_num(*t)).unwrap_or(0);
                    return Err(err(
                        i,
                        if i == 0 {
                            "Int, Float or vector"
                        } else {
                            "Int or Float"
                        },
                    ));
                }
                floats!();
                self.emit(Op::F3(F3::Lerp), span);
                Ok(Ty::Float)
            }
            Fun::Smoothstep => {
                floats!();
                self.emit(Op::F3(F3::Smoothstep), span);
                Ok(Ty::Float)
            }
            Fun::Length | Fun::Normalize => {
                let n = vec_len(t0).ok_or_else(|| err(0, "a vector"))?;
                if matches!(fun, Fun::Length) {
                    self.emit(Op::Length(n), span);
                    Ok(Ty::Float)
                } else {
                    self.emit(Op::Normalize(n), span);
                    Ok(t0)
                }
            }
            Fun::Dot => {
                let n = vec_len(t0).ok_or_else(|| err(0, "a vector"))?;
                if tys[1] != t0 {
                    return Err(err(1, t0.name()));
                }
                self.emit(Op::Dot(n), span);
                Ok(Ty::Float)
            }
            Fun::Cross => {
                if let Some(i) = tys.iter().position(|t| *t != Ty::Vec3) {
                    return Err(err(i, "Vec3"));
                }
                self.emit(Op::Cross, span);
                Ok(Ty::Vec3)
            }
            Fun::Vec(n) => {
                floats!();
                self.emit(Op::MakeVec(n), span);
                Ok(vec_ty(n))
            }
            Fun::Oklch => {
                floats!();
                self.emit(Op::MakeColor(tys.len() == 4), span);
                Ok(Ty::Color)
            }
            Fun::Len | Fun::Upper | Fun::Lower => {
                if t0 != Ty::String {
                    return Err(err(0, "String"));
                }
                let (op, ty) = match fun {
                    Fun::Len => (Op::Len, Ty::Int),
                    Fun::Upper => (Op::Upper, Ty::String),
                    _ => (Op::Lower, Ty::String),
                };
                self.emit(op, span);
                Ok(ty)
            }
            Fun::Str => {
                if t0 != Ty::String {
                    self.emit(Op::StrOf(t0), span);
                }
                Ok(Ty::String)
            }
            Fun::ToInt => match t0 {
                Ty::Int => Ok(Ty::Int),
                Ty::Float => {
                    self.emit(Op::FloatToInt, span);
                    Ok(Ty::Int)
                }
                _ => Err(err(0, "Int or Float")),
            },
            Fun::ToFloat => {
                floats!();
                Ok(Ty::Float)
            }
            Fun::If | Fun::Ch | Fun::Concat => Err(error(
                ErrorKind::Internal("special form reached generic call path"),
                span,
            )),
        }
    }

    #[inline(never)]
    fn gen_ch(&mut self, args: &[Expr], span: Span) -> Result<Ty, Box<ExprError>> {
        let Some(arg) = args.first() else {
            return Err(error(ErrorKind::Internal("ch arity"), span));
        };
        let ExprKind::Str(path) = &arg.kind else {
            return Err(error(ErrorKind::ChannelPathNotLiteral, arg.span));
        };
        let Some(channel) = self.env.resolve(path) else {
            return Err(error(ErrorKind::UnknownChannel(path.clone()), arg.span));
        };
        if !is_expr_type(channel.ty) {
            return Err(error(
                ErrorKind::UnsupportedChannelType {
                    path: path.clone(),
                    ty: channel.ty,
                },
                arg.span,
            ));
        }
        self.deps.push(channel.slot);
        self.emit(Op::Load(channel.slot, channel.ty), span);
        Ok(channel.ty)
    }
}

#[inline(never)]
fn check_bool(t: Ty, span: Span, context: &'static str) -> Result<(), Box<ExprError>> {
    if t == Ty::Bool {
        Ok(())
    } else {
        Err(mismatch(context, "Bool", t, span))
    }
}

/// Resolve a function name and check its arity.
#[inline(never)]
fn check_call(
    name: &str,
    name_span: Span,
    found: usize,
    span: Span,
) -> Result<Fun, Box<ExprError>> {
    let Some((fun, min, max)) = lookup(name) else {
        return Err(error(
            ErrorKind::UnknownFunction(name.to_owned()),
            name_span,
        ));
    };
    if found < min || found > max {
        return Err(error(
            ErrorKind::Arity {
                function: name.to_owned(),
                expected: arity_text(min, max),
                found,
            },
            span,
        ));
    }
    Ok(fun)
}
