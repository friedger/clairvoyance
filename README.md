# Clairvoyance

A symbolic execution engine for [Clarity](https://clarity-lang.org), the smart
contract language of the [Stacks](https://stacks.co) blockchain.

Clairvoyance runs a Clarity function over *symbolic* inputs — exploring every
reachable path at once instead of one concrete call at a time — and checks each
terminating state against a specification you write in the contract's comments.
It reports the paths that violate the spec, the states you did not account for,
and the state writes that differ from what you claimed.

It is built directly on the production Clarity VM (the `clarity` crate from
`stacks-core`), so it reads the same bytes the chain does.

> **Status: early / experimental.** The engine handles a large subset of
> Clarity — arithmetic, booleans, sequences, principals, tuples, options,
> responses, maps, fungible and non-fungible tokens, `as-contract`, and
> post-condition allowances — and can verify specifications on self-contained
> contracts today. Cross-contract `contract-call?`, traits, and some builtins
> are still in progress (see [Limitations](#limitations)). Expect rough edges.

## Building

Clairvoyance vendors `stacks-core` as a submodule and builds against it, so
clone recursively:

```sh
git clone --recurse-submodules https://github.com/jcnelson/clairvoyance
cd clairvoyance
cargo build            # debug build at target/debug/clairvoyance
```

If you already cloned without submodules:

```sh
git submodule update --init --depth 1 stacks-core
```

Requires a recent Rust toolchain (the crate uses edition 2024).

## Quickstart

There is a worked contract in [`examples/counter.clar`](examples/counter.clar):
a counter whose `bump` is correctly specified, whose `reset` carries a
deliberately wrong specification, and whose `peek` has no specification at all.

Check every function in it:

```sh
$ clairvoyance sym check SP000000000000000000002Q6VF78.counter examples/counter.clar --all
Checked 3 function(s) in SP000000000000000000002Q6VF78.counter:

  PASS      bump  (1 state(s))
  VIOLATED  reset
  PASS      peek  (1 state(s))

Summary: 2 passed, 1 violated, 0 spec-error, 0 error, 0 no-spec
```

The command exits non-zero when anything it checked failed, so it drops
straight into CI. Re-run on a single function to see the full report:

```sh
$ clairvoyance sym check SP000000000000000000002Q6VF78.counter examples/counter.clar reset
VIOLATED: `reset` does not satisfy its (@clairvoyance ...) specification.

Incorrect var-set:
      Path: reset.return
   Formula: (ok true)
  Variable: SP000000000000000000002Q6VF78.counter.count
  Expected: u0
     Given: u1
```

`CODE` may be a file path or `-` to read the contract from stdin.

## Commands

Run `clairvoyance help`, `clairvoyance sym help`, or `clairvoyance contract help`
for the full option list.

| command | what it does |
| --- | --- |
| `sym check CONTRACT_ID CODE [FUNCTION]` | Verify a function against its specification and report **PASS / VIOLATED / SPEC ERROR / NO SPEC**, with a meaningful exit code. Omit `FUNCTION`, or pass `--all`, to check every public and read-only function and print a summary. |
| `sym induct CONTRACT_ID CODE` | Inductive invariant checking: for each `(invariant, mutator)` pair, assume the invariant, run the mutator, and check it still holds. Reports **HOLDS / NOT PROVEN / VIOLATED**. Uses an SMT solver for the residuals when one is installed (`--solver PATH`, `--no-smt`). |
| `sym exec-func CONTRACT_ID CODE FUNCTION` | Symbolically execute a function and print every terminating state — its path predicate, return value, and state writes. Also enforces any specification. |
| `sym reachable CONTRACT_ID CODE FUNCTION` | Print the call graph reachable from a function: which data vars and maps it may read and write, transitively. |
| `contract ast\|context\|analyze CONTRACT_ID CODE` | Inspect the parsed AST, the contract context, or the analysis of a contract. |

Common options: `--dep CONTRACT_ID:PATH` loads a dependency contract (repeatable,
instantiated in order); `--concretized-trait C.f.v:IMPL` binds a trait argument
to a concrete implementation for dynamic dispatch; `--tx-sender`,
`--contract-caller`, and `--tx-sponsor` fix those runtime values (each defaults
to a fresh symbol); `-v` / `-vv` turn up the engine's log level (quiet by
default).

## Writing a specification

A specification lives in a Clarity comment directly above the function, inside a
`(@clairvoyance ...)` block. The engine reads it off the function's comments,
runs the function, and checks it. A function with no block is explored but not
judged (reported as `NO SPEC`).

The two core commands are `invariant` and `halt`:

```clarity
;; (@clairvoyance
;;     ;; Every state that returns (err u0) must have taken the odd branch.
;;     (invariant (err u0)
;;         (not (is-eq (mod (x uint) u2) u0)))
;;     ;; The even branch returns (ok true) and inserts (x -> x) into `m`.
;;     (halt
;;         (result (ok true))
;;         (condition (is-eq (mod (x uint) u2) u0))
;;         (map-write 'SP8H248...ARTQ82.contract.m (x uint) (x uint))))
(define-public (set-if-even (x uint))
    (if (is-eq (mod x u2) u0)
        (ok (map-insert m x x))
        (err u0)))
```

- A free input is written `(name type)`, e.g. `(x uint)`.
- `(invariant RESULT CONCLUSION)` requires that every terminating state whose
  return value is `RESULT` has a path predicate that implies `CONCLUSION`.
- `(halt ...)` is the same, plus the exact state the matching path must leave
  behind: `(var-write ...)`, `(map-write ...)`, `(map-delete ...)`,
  `(early-return)`, `(panicking)`, and the `reachable-*` sets.
- To name a var or map entry's value *on entry* (before the call ran), use
  `(loaded-var 'ADDR.c.v (v uint))` rather than `(var-get ...)`.

The full grammar is in
[`clairvoyance/src/sym/command.clar`](clairvoyance/src/sym/command.clar).

The engine reports four kinds of failure, so a mismatch tells you *which* half is
wrong:

| report | meaning |
| --- | --- |
| unchecked continuation | a reachable terminating state no command accounted for |
| unmatched halting condition | a command that matched no terminating state |
| halting condition failed | a matched state whose predicate does not imply the conclusion |
| incorrect / missing / unchecked var or map write | a state write that differs from, is absent from, or is not covered by the spec |

## Inductive invariant checking

`sym induct` checks that a contract's own invariants are *preserved* by its
mutators, which is the inductive step of a safety proof. It follows the
convention that an invariant is a read-only function named `invariant-*`
returning `bool` (the same convention the property-based fuzzers use).

For each `(invariant I, mutator M)` pair it synthesizes a harness that assumes
`I` on entry, runs `M` over fresh symbolic arguments, and asserts `I` still
holds, then symbolically executes it:

```sh
$ clairvoyance sym induct SP000000000000000000002Q6VF78.ledger examples/inductive.clar
invariant-a-eq-b
  HOLDS      bump-both
  NOT PROVEN bump-a  (fails when: (and (is-eq a b) (not (is-eq (+ a u1) b))))
  HOLDS      bump-c
```

- **HOLDS** — the engine proved `I` holds after `M` on every path (the path
  where it could fail was shown to be unreachable).
- **NOT PROVEN** — the engine could not rule out a path where `I` fails after
  `M`; the residual condition is printed. This is *either* a real conditional
  violation (a condition you can satisfy — `bump-a` above breaks `a == b`
  whenever it held) *or* a term neither the simplifier nor the solver could
  reduce. The printed condition tells you which to suspect: if you can read a
  counterexample out of it, it is the first kind.
- **VIOLATED** — `I` fails after `M` unconditionally.
- **UNFINISHED** — the pair ran past its time budget (`--time-budget`,
  60s by default) or step budget (`--max-steps`) before the engine was done.
  It is counted with the not-proven, because it is: the tool stopped looking,
  which is never evidence that an invariant holds.

This works by composing the mutator's state effects into the invariant's reads
— a called function's data-var reads now resolve against the caller's current
state, so a value the mutator writes flows into the invariant that reads it.

### Discharging the residual with an SMT solver

A NOT PROVEN residual is a formula, and most of the interesting ones are false
— they just need more arithmetic than an algebraic simplifier has. If a solver is
on your PATH, `sym induct` hands each residual to it and upgrades the result to
HOLDS when the solver proves the failing path is infeasible:

```sh
$ clairvoyance sym induct SP000000000000000000002Q6VF78.smt examples/inductive-smt.clar
decided by: simplifier + z3

invariant-count-even
  HOLDS      add-two  (by solver)
  NOT PROVEN add-one  (fails when: (and (is-eq (mod count u2) u0) ...))

invariant-sq-eq-n-squared
  HOLDS      inc-n  (by solver)

Summary: 5 holds (2 by solver), 0 violated, 1 not-proven, 0 skipped
```

`add-two` keeps `count` even and `inc-n` keeps `sq = n^2`; proving them needs
modular arithmetic and the nonlinear identity `(n+1)^2 = n^2 + 2n + 1`
respectively, neither of which the simplifier can do. `add-one` genuinely breaks
the invariant, so it stays NOT PROVEN with its counterexample condition. Pass
`--no-smt` to see all three fall back.

- Any SMT-LIB 2 solver works. [z3](https://github.com/Z3Prover/z3) is the
  default (`brew install z3`, `apt install z3`, or `pipx install z3-solver`);
  cvc5 is tried next. Override with `--solver PATH` or `$CLAIRVOYANCE_SMT`.
- **Only `unsat` is believed.** The translation to SMT is an over-approximation:
  anything it cannot model becomes a fresh uninterpreted constant, which only
  ever makes the formula easier to satisfy. So a solver `unsat` is a real proof
  of infeasibility, while `sat` proves nothing and is reported as NOT PROVEN,
  exactly as it would be without a solver. The solver can turn NOT PROVEN into
  HOLDS; it can never turn HOLDS into a violation, and a missing solver never
  changes an answer from correct to wrong.
- Each query gets a 5-second timeout; a timeout, a crash, or a broken pipe all
  read as "not proven".

## Limitations

Clairvoyance is young, and it is honest about what it cannot yet do:

- **Cross-contract calls compose against provided contracts.** A
  `(contract-call? .other f ...)` to a contract loaded with `--dep` is evaluated
  into and its state effects compose back, so `sym induct` can check invariants
  that span contracts (see `examples/ledger.clar`, whose invariant reads a
  token's supply across the boundary). Traits are dispatched with
  `--concretized-trait`. The limit is a callee that is *not* provided: the
  Clarity analyzer needs it present to type-check the call, so a contract that
  calls `pox-5` or the sBTC contracts must be given those as deps (a signature
  stub is enough) — they cannot be treated as opaque.
- **The solver is consulted, not relied on.** `sym induct` discharges residual
  conditions with an SMT solver when one is installed, but the rest of the
  engine — path feasibility during execution, and all of `sym check` — is
  still decided by the algebraic simplifier alone. When the simplifier cannot
  reduce a term it leaves it unsimplified and carries on, rather than aborting,
  so a hard term shows up as an unresolved formula, not a crash.
- **Some builtins are unmodelled**, including the Bitcoin transaction reader
  (`get-bitcoin-tx-output?`) and the signature-verification builtins.
- **Cross-function composition covers data vars and concrete map keys.** A
  callee's data-var reads/writes and its `map-get?` of a key the caller wrote
  compose into the caller. What is not resolved is a read of an *uninitialized*
  slot the caller did not write, and symbolic-key aliasing, so `sym induct`
  reasons best about invariants over data vars and fixed map keys.
- **Not every term reaches the solver.** The translation to SMT is a deliberate
  over-approximation: an operation it does not model (hashes, `sqrti`, signed
  division, buffers and sequences) becomes a fresh uninterpreted constant. That
  is what keeps `Unsat` trustworthy, but it also means an invariant that turns
  on one of those reads as NOT PROVEN no matter which solver you point at it.

If the engine cannot evaluate a function it says so and exits non-zero, rather
than reporting a false pass.

## Repository layout

```
clairvoyance/src/
  main.rs             entry point and log configuration
  cli/                the command-line front end (sym, contract)
  core/               contract loading, the Error type, the proof-failure report
  smt/                SMT-LIB translation and the solver subprocess
  sym/                the symbolic engine: SymOp, the simplifier, continuations
    command.rs        the (@clairvoyance ...) command interpreter
    command.clar      the command-language reference grammar
  tests/              unit and command tests
examples/             worked example contracts
```

## License

GNU Affero General Public License v3.0. See [LICENSE](LICENSE).
