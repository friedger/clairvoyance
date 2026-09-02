// Copyright (C) 2026 Trust Machines
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <http://www.gnu.org/licenses/>.

//! An SMT backend: decide satisfiability of a path predicate with a real
//! solver, where the algebraic simplifier gives out.
//!
//! # What this is for
//!
//! The simplifier can normalise and cancel terms, but it is not a decision
//! procedure: it cannot tell you that `(and (is-eq (mod x u2) u0) (not (is-eq
//! (mod (+ x u2) u2) u0)))` is unsatisfiable. A solver can. This module
//! translates a [`Predicate`] into SMT-LIB and asks.
//!
//! # How the translation stays sound
//!
//! The translation is deliberately an *over-approximation*: anything it cannot
//! model faithfully becomes a fresh uninterpreted constant, which only ever
//! admits **more** models than the real term does. Consequently:
//!
//! * `Unsat` is trustworthy. If the relaxed formula has no model, neither does
//!   the real one, so the path really is infeasible.
//! * `Sat` is **not** a proof of feasibility -- the model may rely on a
//!   freedom the relaxation invented.
//!
//! Callers must therefore only ever act on `Unsat`, and treat `Sat` exactly
//! like `Unknown`. That is what keeps the solver from ever turning a sound
//! answer into an unsound one: it can strengthen a result, never weaken it.
//!
//! # Modelling Clarity's integers
//!
//! `int` and `uint` are translated to the SMT `Int` sort -- unbounded
//! mathematical integers -- rather than 128-bit bitvectors. That is the
//! faithful choice for Clarity, whose arithmetic *aborts* on overflow instead
//! of wrapping: a program that would wrap has no continuation to reason about,
//! so wrap-around behaviour is not something the formula should be able to
//! express. It is also far better for the nonlinear reasoning this exists to
//! do. Every uninterpreted constant standing for a `uint` gets a `>= 0`
//! assumption, which is a true fact about the term and so sound to assert.
//!
//! `div` and `mod` are only translated when both operands are known-unsigned,
//! because Clarity truncates toward zero for signed values while SMT-LIB's
//! `div`/`mod` are Euclidean; for signed operands the term becomes
//! uninterpreted instead of subtly wrong.

use std::collections::HashMap;
use std::env;
use std::process::Command;
use std::process::Stdio;

use easy_smt::{Context, ContextBuilder, Response, SExpr};

use clarity_types::Value;

use crate::sym::{Predicate, SymOp};

/// Largest exponent expanded into repeated multiplication. Squares and cubes
/// are what invariants actually use; past that the term is big enough to hurt
/// the solver more than the extra precision helps.
const MAX_EXPANDED_EXPONENT: u128 = 4;

/// What a solver concluded about a formula.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// Provably has no model. Sound: the path is infeasible.
    Unsat,
    /// A model was found for the *relaxed* formula. Not a proof of anything;
    /// treat as `Unknown`.
    Sat,
    /// No solver, a solver error, or the solver gave up.
    Unknown,
}

/// A solver to talk to: the program and the arguments that put it in
/// SMT-LIB-2-over-stdin mode.
#[derive(Debug, Clone)]
pub struct Solver {
    pub program: String,
    pub args: Vec<String>,
}

fn runnable(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Find a solver: `CLAIRVOYANCE_SMT` if set, else the first of z3 or cvc5 that
/// runs. `None` when there is none, in which case every query is `Unknown` and
/// the tool behaves exactly as it does without a solver.
pub fn find_solver() -> Option<Solver> {
    if let Ok(program) = env::var("CLAIRVOYANCE_SMT") {
        if !program.is_empty() {
            return Some(Solver { args: default_args(&program), program });
        }
    }
    for program in ["z3", "cvc5"] {
        if runnable(program) {
            return Some(Solver { program: program.into(), args: default_args(program) });
        }
    }
    None
}

/// A solver at an explicit path, or `None` if it does not run.
pub fn solver_at(program: &str) -> Option<Solver> {
    runnable(program).then(|| Solver {
        program: program.to_string(),
        args: default_args(program),
    })
}

/// The flags that put a given solver in SMT-LIB-2-over-stdin mode. Recognised
/// by the executable's name, since that is all an override gives us; anything
/// unrecognised gets z3's flags, which most solvers accept.
fn default_args(program: &str) -> Vec<String> {
    let name = std::path::Path::new(program)
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let args: &[&str] = if name.contains("cvc") {
        &["--lang=smt2", "--incremental"]
    } else if name.contains("yices") {
        &["--incremental"]
    } else {
        &["-smt2", "-in"]
    };
    args.iter().map(|s| s.to_string()).collect()
}

/// Translates `SymOp` into SMT-LIB, inventing uninterpreted constants for
/// everything it cannot model.
struct Encoder<'a> {
    ctx: &'a mut Context,
    /// Uninterpreted constants, keyed by (is-boolean, the term's printed form)
    /// so the *same* term always maps to the *same* constant -- which is what
    /// makes `x` and `(not x)` contradict each other even when `x` is opaque.
    leaves: HashMap<(bool, String), SExpr>,
    /// Facts that are true of the leaves, e.g. a `uint` is non-negative.
    side: Vec<SExpr>,
    next: usize,
    failed: bool,
}

impl<'a> Encoder<'a> {
    fn new(ctx: &'a mut Context) -> Self {
        Self { ctx, leaves: HashMap::new(), side: vec![], next: 0, failed: false }
    }

    fn num_u(&self, v: u128) -> SExpr {
        self.ctx.atom(v.to_string())
    }

    fn num_i(&self, v: i128) -> SExpr {
        if v < 0 {
            let magnitude = self.ctx.atom(v.unsigned_abs().to_string());
            self.ctx.negate(magnitude)
        } else {
            self.ctx.atom(v.to_string())
        }
    }

    /// An uninterpreted constant standing for a term we do not model.
    fn leaf(&mut self, op: &SymOp, is_bool: bool) -> SExpr {
        let key = (is_bool, op.to_string());
        if let Some(existing) = self.leaves.get(&key) {
            return *existing;
        }
        let name = format!("clv_{}", self.next);
        self.next += 1;
        let sort = if is_bool { self.ctx.bool_sort() } else { self.ctx.int_sort() };
        let constant = match self.ctx.declare_const(name, sort) {
            Ok(c) => c,
            Err(_) => {
                self.failed = true;
                self.ctx.true_()
            }
        };
        if !is_bool && op.is_unsigned() == Some(true) {
            // A `uint` is non-negative. True of the real term, so sound.
            let zero = self.ctx.atom("0");
            let nonneg = self.ctx.gte(constant, zero);
            self.side.push(nonneg);
        }
        self.leaves.insert(key, constant);
        constant
    }

    fn all_unsigned(ops: &[Box<SymOp>]) -> bool {
        ops.iter().all(|op| op.is_unsigned() == Some(true))
    }

    fn ints(&mut self, ops: &[Box<SymOp>]) -> Vec<SExpr> {
        let mut out = Vec::with_capacity(ops.len());
        for op in ops.iter() {
            out.push(self.to_int(op));
        }
        out
    }

    fn bools(&mut self, ops: &[Box<SymOp>]) -> Vec<SExpr> {
        let mut out = Vec::with_capacity(ops.len());
        for op in ops.iter() {
            out.push(self.to_bool(op));
        }
        out
    }

    /// Does this term denote a boolean? Used to pick the sort for `is-eq`.
    fn looks_bool(op: &SymOp) -> bool {
        matches!(
            op,
            SymOp::Constant(Value::Bool(_))
                | SymOp::And(..)
                | SymOp::Or(..)
                | SymOp::Not(..)
                | SymOp::Greater(..)
                | SymOp::Geq(..)
                | SymOp::Leq(..)
                | SymOp::Less(..)
                | SymOp::Equals(..)
                | SymOp::IsOkay(..)
                | SymOp::IsErr(..)
                | SymOp::IsSome(..)
                | SymOp::IsNone(..)
        ) || matches!(op, SymOp::If(_, a, b) if Self::looks_bool(a) || Self::looks_bool(b))
          || matches!(op, SymOp::Named(_, def) if Self::looks_bool(def))
    }

    /// A named formula: an uninterpreted constant like any other leaf, plus
    /// the fact that it equals its definition, asserted once per name.
    fn named(&mut self, op: &SymOp, def: &SymOp, is_bool: bool) -> SExpr {
        let key = (is_bool, op.to_string());
        if let Some(existing) = self.leaves.get(&key) {
            return *existing;
        }
        let constant = self.leaf(op, is_bool);
        let value = if is_bool { self.to_bool(def) } else { self.to_int(def) };
        let defined = self.ctx.eq_many(vec![constant, value]);
        self.side.push(defined);
        constant
    }

    fn to_int(&mut self, op: &SymOp) -> SExpr {
        match op {
            SymOp::Constant(Value::UInt(v)) => self.num_u(*v),
            SymOp::Constant(Value::Int(v)) => self.num_i(*v),
            SymOp::Add(ops) => {
                let xs = self.ints(ops);
                self.ctx.plus_many(xs)
            }
            SymOp::Subtract(ops) => {
                // Clarity aborts on unsigned underflow; SMT `-` may go
                // negative. That is a relaxation, which keeps `Unsat` sound.
                let xs = self.ints(ops);
                self.ctx.sub_many(xs)
            }
            SymOp::Multiply(ops) => {
                let xs = self.ints(ops);
                self.ctx.times_many(xs)
            }
            // `div`/`mod` agree with Clarity only on non-negative operands.
            SymOp::Divide(ops) if Self::all_unsigned(ops) => {
                let xs = self.ints(ops);
                self.ctx.div_many(xs)
            }
            SymOp::Modulo(a, b)
                if a.is_unsigned() == Some(true) && b.is_unsigned() == Some(true) =>
            {
                let x = self.to_int(a);
                let y = self.to_int(b);
                self.ctx.modulo(x, y)
            }
            // The simplifier folds `(* n n)` into a power, so leaving powers
            // opaque would hide ordinary polynomial arithmetic from the solver.
            // A small constant exponent expands to repeated multiplication,
            // which is exact; `^` is not portable SMT-LIB and a symbolic
            // exponent is not worth guessing at, so those stay uninterpreted.
            SymOp::Power(base, exponent) => {
                let literal = match &**exponent {
                    SymOp::Constant(Value::UInt(k)) => Some(*k),
                    SymOp::Constant(Value::Int(k)) if *k >= 0 => Some(*k as u128),
                    _ => None,
                };
                match literal {
                    Some(0) => self.ctx.atom("1"),
                    Some(k) if k <= MAX_EXPANDED_EXPONENT => {
                        let factor = self.to_int(base);
                        self.ctx.times_many(vec![factor; k as usize])
                    }
                    _ => self.leaf(op, false),
                }
            }
            SymOp::If(c, a, b) => {
                let c = self.to_bool(c);
                let a = self.to_int(a);
                let b = self.to_int(b);
                self.ctx.ite(c, a, b)
            }
            SymOp::Named(_, def) => self.named(op, def, false),
            other => self.leaf(other, false),
        }
    }

    fn to_bool(&mut self, op: &SymOp) -> SExpr {
        match op {
            SymOp::Constant(Value::Bool(true)) => self.ctx.true_(),
            SymOp::If(c, a, b) => {
                let c = self.to_bool(c);
                let a = self.to_bool(a);
                let b = self.to_bool(b);
                self.ctx.ite(c, a, b)
            }
            SymOp::Named(_, def) => self.named(op, def, true),
            SymOp::Constant(Value::Bool(false)) => self.ctx.false_(),
            SymOp::And(ops) => {
                let xs = self.bools(ops);
                self.ctx.and_many(xs)
            }
            SymOp::Or(ops) => {
                let xs = self.bools(ops);
                self.ctx.or_many(xs)
            }
            SymOp::Not(inner) => {
                let x = self.to_bool(inner);
                self.ctx.not(x)
            }
            // Each of these is exactly the negation of its partner, so encode
            // the pair through one leaf: `(is-none x)` is `(not (is-some x))`,
            // and `(is-err x)` is `(not (is-ok x))`. Two opaque leaves would
            // let the solver set both true.
            SymOp::IsNone(inner) => {
                let some = SymOp::IsSome(inner.clone());
                let x = self.leaf(&some, true);
                self.ctx.not(x)
            }
            SymOp::IsErr(inner) => {
                let ok = SymOp::IsOkay(inner.clone());
                let x = self.leaf(&ok, true);
                self.ctx.not(x)
            }
            SymOp::Greater(a, b) => {
                let (x, y) = (self.to_int(a), self.to_int(b));
                self.ctx.gt(x, y)
            }
            SymOp::Geq(a, b) => {
                let (x, y) = (self.to_int(a), self.to_int(b));
                self.ctx.gte(x, y)
            }
            SymOp::Leq(a, b) => {
                let (x, y) = (self.to_int(a), self.to_int(b));
                self.ctx.lte(x, y)
            }
            SymOp::Less(a, b) => {
                let (x, y) = (self.to_int(a), self.to_int(b));
                self.ctx.lt(x, y)
            }
            SymOp::Equals(ops) => {
                if ops.iter().all(|o| Self::looks_bool(o)) {
                    let xs = self.bools(ops);
                    self.ctx.eq_many(xs)
                } else {
                    let xs = self.ints(ops);
                    self.ctx.eq_many(xs)
                }
            }
            other => self.leaf(other, true),
        }
    }
}

/// Is `predicate` unsatisfiable? Only `Unsat` is a proof; see the module docs.
///
/// `None` solver, a broken pipe, or a solver that gives up all read as
/// `Unknown`, so a caller that only acts on `Unsat` behaves identically with
/// and without a solver installed.
pub fn predicate_is_unsat(predicate: &Predicate, solver: &Solver) -> Answer {
    let mut builder = ContextBuilder::new();
    builder.solver(&solver.program).solver_args(&solver.args);
    // `CLAIRVOYANCE_SMT_DUMP=dir` keeps every query as SMT-LIB in that
    // directory, one file per query, for looking at what the solver was asked.
    if let Ok(dir) = std::env::var("CLAIRVOYANCE_SMT_DUMP") {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let path = std::path::Path::new(&dir).join(format!("query-{n:04}.smt2"));
        builder.replay_file(std::fs::File::create(path).ok());
    }
    let mut ctx = match builder.build() {
        Ok(ctx) => ctx,
        Err(e) => {
            debug!("SMT: could not start {}: {e}", &solver.program);
            return Answer::Unknown;
        }
    };

    // Nonlinear integer arithmetic is undecidable in general, so bound the
    // effort: an answer we do not get in time is `Unknown`, which costs the
    // caller nothing.
    let timeout = ctx.atom("5000");
    let _ = ctx.set_option(":timeout", timeout);
    let _ = ctx.set_logic("ALL");

    let formula = predicate.clone().as_symop();
    let (encoded, side, failed) = {
        let mut encoder = Encoder::new(&mut ctx);
        let encoded = encoder.to_bool(&formula);
        (encoded, std::mem::take(&mut encoder.side), encoder.failed)
    };
    if failed {
        return Answer::Unknown;
    }

    for fact in side {
        if ctx.assert(fact).is_err() {
            return Answer::Unknown;
        }
    }
    if ctx.assert(encoded).is_err() {
        return Answer::Unknown;
    }

    let answer = match ctx.check() {
        Ok(Response::Unsat) => Answer::Unsat,
        Ok(Response::Sat) => Answer::Sat,
        Ok(Response::Unknown) => Answer::Unknown,
        Err(e) => {
            debug!("SMT: solver error: {e}");
            Answer::Unknown
        }
    };
    debug!("SMT: {answer:?} for {predicate}");
    answer
}
