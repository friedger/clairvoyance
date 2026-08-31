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

use crate::sym::Symbex;

use crate::sym::command::{Command, CommandContext};
use crate::core::BackingStore;
use crate::core::Error;
use crate::tests::default_contract_id;

use clarity_types::types::QualifiedContractIdentifier;
use clarity_types::types::StandardPrincipalData;
use clarity_types::types::PrincipalData;
use clarity_types::types::signatures::{TypeSignature as TS};

use crate::tests::*;

#[test]
fn test_extract_command_programs() {
    let tests = vec![
        (
            "this is a normal comment",
            vec![]
        ),
        (
            "(@clairvoyance (test \"this is a program\"))",
            vec!["( test \"this is a program\" )"],
        ),
        (
            "this is a normal comment (@clairvoyance (test \"with a program\")) and a trailer",
            vec!["( test \"with a program\" )"]
        ),
        (
            "(@clairvoyance )",
            vec![]
        ),
        (
            "(@clairvoy-this-is-a-normal-comment)",
            vec![]
        ),
        (
            "(this is a normal comment (@clairvoyance (test \"and this is a program\")))",
            vec!["( test \"and this is a program\" )"],
        ),
        (
            "(@clairvoyance (test \"can nest (@clairvoyance (test \"programs and quotes\"))\")) and have trailers",
            vec!["( test \"can nest (@clairvoyance (test \"programs and quotes\"))\" )"]
        ),
        (
            r#"
            (@clairvoyance (test "drop end-of-file comments")) ;; this is a comment
            "#,
            vec!["( test \"drop end-of-file comments\" )"]
        ),
        (
            r#"
            (@clairvoyance
                (test "drop end-of-line comments") ;; this is a comment
            )"#,
            vec!["( test \"drop end-of-line comments\" )"]
        ),
        (
            r#"
            ;; this is a comment
            (@clairvoyance (test "comments do not break end-of-program) ;; )
            ")) ;; this is a comment
            ;; this is a comment
            "#,
            vec!["( test \"comments do not break end-of-program) ;; )\n            \" )"]
        ), 
        (
            r#"
            (@clairvoyance
                (test
                    ;; this is a comment!
                    "can nest comments"))
            "#,
            vec!["( test \"can nest comments\" )"]
        ),
        (
            r#"
            ((((((((@clairvoyance (test "this is processed"))
            "#,
            vec!["( test \"this is processed\" )"],
        ),
        (
            r#"
            " (@clairvoyance (test "this is processed"))
            "#,
            vec!["( test \"this is processed\" )"],
        ),
        (
            // comments are only ignored _within_ (@clairvoyance ..) s-exps
            r#"
            ;; (@clairvoyance (test "this is processed"))
            "#,
            vec!["( test \"this is processed\" )"],
        ),
    ];

    for (inp, out) in tests.into_iter() {
        let out : Vec<_> = out.into_iter().map(|s| s.to_string()).collect();
        let progs = CommandContext::extract_command_programs(&inp);
        assert_eq!(out, progs, "Failed to parse `{inp}`");
    }
}

#[test]
fn test_eval_program() {
    let tests = vec![
        (
            r#"(test "hello world!")"#,
            Ok(vec![Command::Test("\"hello world!\"".to_string())]),
        ),
        (
            r#"
                (test "foo")
                (test "bar")
            "#,
            Ok(vec![
               Command::Test("\"foo\"".to_string()),
               Command::Test("\"bar\"".to_string()),
            ])
        ),
        (
            r#"
                (test u1)
                (test true)
                (test tx-sender)
            "#,
            Ok(vec![
               Command::Test("u1".to_string()),
               Command::Test("true".to_string()),
               Command::Test("(tx-sender principal)".to_string()),
            ]),
        ),
        (
            r#"
                (invariant
                    (ok (map-entry 'SP8H248H248H248H248H248H248H248H24ARTQ82.contract.m (modulus uint)))
                    (is-eq (len (items (list 5 uint))) u0))
            "#,
            Ok(vec![
                Command::Invariant(
                   *ok(fq_map_get(&QualifiedContractIdentifier::parse("SP8H248H248H248H248H248H248H248H24ARTQ82.contract").unwrap(), "m", vu("modulus"))),
                   eq(llen(vl("items", TS::UIntType, 5)), cu(0)).try_as_predicate().unwrap()
                )
            ])
        ),
        (
            r#"(test "force-failure!")"#,
            Err("`test` command forced to fail"),
        )
    ];

    let mut ctx = CommandContext::new();
    for (prog, expected_events) in tests.into_iter() {
        match ctx.eval_program(&prog, 0) {
            Ok(events) => {
                let Ok(expected_events) = expected_events else {
                    panic!("Evaluating program `{prog}` was supposed to fail (got Ok event `{events:?}`)");
                };
                assert_eq!(events, expected_events, "Failed to run program {prog}");
            }
            Err(Error::Program(program_error)) => {
                let msg = &program_error.cause;
                let Err(expected_msg) = expected_events else {
                    panic!("Evaluating program `{prog}` was not supposed to fail (got msg `{msg}`)");
                };
                assert!(msg.find(expected_msg).is_some(), "Failed to find expected message `{expected_msg}` in `{msg}`");
            }
            Err(e) => {
                panic!("Unexpected error: `{e:?}`");
            }
        }
    }
}

#[test]
fn test_eval_invariant() {
    let contract_id = QualifiedContractIdentifier::parse("SP8H248H248H248H248H248H248H248H24ARTQ82.foo").unwrap();

    let tests : Vec<(&str, Result<Vec<Command>, Error>)> = vec![
        (
            "(invariant true true)",
            Ok(vec![Command::Invariant(*t(), *pt())]),
        ),
        (
            "(invariant u0 true)",
            Ok(vec![Command::Invariant(*cu(0), *pt())]),
        ),
        (
            "(invariant 0 true)",
            Ok(vec![Command::Invariant(*ci(0), *pt())]),
        ),
        (
            "(invariant (list u5) true)",
            Ok(vec![Command::Invariant(*lcons(vec![cu(5)]), *pt())]),
        ),
        (
            "(invariant (tuple (x u3)) true)",
            Ok(vec![Command::Invariant(*tcons(vec![("x", cu(3))]), *pt())]),
        ),
        (
            "(invariant { y: u4 } true)",
            Ok(vec![Command::Invariant(*tcons(vec![("y", cu(4))]), *pt())]),
        ),
        (
            "(invariant 'SP8H248H248H248H248H248H248H248H24ARTQ82 true)",
            Ok(vec![Command::Invariant(*cp(PrincipalData::parse("SP8H248H248H248H248H248H248H248H24ARTQ82").unwrap()), *pt())]),
        ),
        (
            "(invariant 'SP8H248H248H248H248H248H248H248H24ARTQ82.foo true)",
            Ok(vec![Command::Invariant(*cp(PrincipalData::parse("SP8H248H248H248H248H248H248H248H24ARTQ82.foo").unwrap()), *pt())]),
        ),
        (
            "(invariant (ok true) true)",
            Ok(vec![Command::Invariant(*ok(cb(true)), *pt())]),
        ),
        (
            "(invariant (err false) true)",
            Ok(vec![Command::Invariant(*err(cb(false)), *pt())]),
        ),
        (
            "(invariant (some true) true)",
            Ok(vec![Command::Invariant(*some(cb(true)), *pt())]),
        ),
        (
            "(invariant 0x112233 true)",
            Ok(vec![Command::Invariant(*csb(vec![0x11, 0x22, 0x33]), *pt())]),
        ),
        (
            "(invariant \"hello world\" true)",
            Ok(vec![Command::Invariant(*cssa("hello world"), *pt())]),
        ),
        (
            "(invariant u\"hello world\" true)",
            Ok(vec![Command::Invariant(*cssu("hello world"), *pt())]),
        ),
        (
            "(invariant (x uint) true)",
            Ok(vec![Command::Invariant(*vu("x"), *pt())]),
        ),
        (
            "(invariant (x int) true)",
            Ok(vec![Command::Invariant(*vi("x"), *pt())]),
        ),
        (
            "(invariant (x bool) true)",
            Ok(vec![Command::Invariant(*vb("x"), *pt())]),
        ),
        (
            "(invariant (x (optional uint)) true)",
            Ok(vec![Command::Invariant(*vo("x", TS::UIntType), *pt())]),
        ),
        (
            "(invariant (loaded-var 'SP8H248H248H248H248H248H248H248H24ARTQ82.foo.x (x uint)) true)",
            Ok(vec![Command::Invariant(*fqlv(&contract_id, "x", vu("x")), *pt())]),
        ),
        (
            "(invariant (loaded-var-const 'SP8H248H248H248H248H248H248H248H24ARTQ82.foo.x u1) true)",
            Ok(vec![Command::Invariant(*fqlv(&contract_id, "x", cu(1)), *pt())]),
        ),
        (
            "(invariant (loaded-var-type 'SP8H248H248H248H248H248H248H248H24ARTQ82.foo.x uint) true)",
            Ok(vec![Command::Invariant(*fqlv(&contract_id, "x", vu("x")), *pt())]),
        ),
        (
            "(invariant (loaded-var-sym 'SP8H248H248H248H248H248H248H248H24ARTQ82.foo.x (x uint)) true)",
            Ok(vec![Command::Invariant(*fqlv(&contract_id, "x", vu("x")), *pt())]),
        ),
        (
            "(invariant (+ (x uint) (y uint) (z uint)) true)",
            Ok(vec![Command::Invariant(*add(vec![vu("x"), vu("y"), vu("z")]), *pt())]),
        ),
        (
            "(invariant (- (x uint) (y uint) (z uint)) true)",
            Ok(vec![Command::Invariant(*sub(vec![vu("x"), vu("y"), vu("z")]), *pt())]),
        ),
        (
            "(invariant (* (x uint) (y uint) (z uint)) true)",
            Ok(vec![Command::Invariant(*mul(vec![vu("x"), vu("y"), vu("z")]), *pt())]),
        ),
        (
            "(invariant (/ (x uint) (y uint) (z uint)) true)",
            Ok(vec![Command::Invariant(*div(vec![vu("x"), vu("y"), vu("z")]), *pt())]),
        ),
        (
            "(invariant (mod (x uint) (y uint)) true)",
            Ok(vec![Command::Invariant(*rem(vu("x"), vu("y")), *pt())]),
        ),
        (
            "(invariant (and (x bool) (y bool) (z bool)) true)",
            Ok(vec![Command::Invariant(*and(vec![vb("x"), vb("y"), vb("z")]), *pt())]),
        ),
        (
            "(invariant (or (x bool) (y bool) (z bool)) true)",
            Ok(vec![Command::Invariant(*or(vec![vb("x"), vb("y"), vb("z")]), *pt())]),
        ),
        (
            "(invariant (map-entry 'SP8H248H248H248H248H248H248H248H24ARTQ82.foo.m (x uint)) true)",
            Ok(vec![Command::Invariant(*fq_map_get(&contract_id, "m", vu("x")), *pt())])
        ),
        (
            "(invariant (map-entry 'SP8H248H248H248H248H248H248H248H24ARTQ82.foo.m (x uint) (y bool)) true)",
            Ok(vec![Command::Invariant(*fqlm(&contract_id, "m", vu("x"), vb("y")), *pt())])
        ),
        (
            "(invariant (map-entry-const 'SP8H248H248H248H248H248H248H248H24ARTQ82.foo.m (x uint) true) true)",
            Ok(vec![Command::Invariant(*fqlm(&contract_id, "m", vu("x"), cb(true)), *pt())])
        ),
        (
            "(invariant (map-entry-type 'SP8H248H248H248H248H248H248H248H24ARTQ82.foo.m (x uint) (y bool) bool) true)",
            Ok(vec![Command::Invariant(*fqlm(&contract_id, "m", vu("x"), vb("y")), *pt())])
        ),
        (
            "(invariant (map-entry-sym 'SP8H248H248H248H248H248H248H248H24ARTQ82.foo.m (x uint)) true)",
            Ok(vec![Command::Invariant(*fq_map_get(&contract_id, "m", vu("x")), *pt())])
        ),
        (
            "(invariant (map-entry-sym 'SP8H248H248H248H248H248H248H248H24ARTQ82.foo.m (x uint) (y bool)) true)",
            Ok(vec![Command::Invariant(*fqlm(&contract_id, "m", vu("x"), vb("y")), *pt())])
        ),
    ];

    let mut ctx = CommandContext::new();
    for (prog, expected_events) in tests.into_iter() {
        match ctx.eval_program(&prog, 0) {
            Ok(events) => {
                let Ok(expected_events) = expected_events else {
                    panic!("Evaluating program `{prog}` was supposed to fail (got Ok event `{events:?}`)");
                };
                assert_eq!(events, expected_events, "Failed to run program {prog}");
            }
            Err(Error::Program(program_error)) => {
                let msg = &program_error.cause;
                let Err(Error::Program(expected_program_error)) = expected_events else {
                    panic!("Evaluating program `{prog}` was not supposed to fail (got msg `{msg}`)");
                };
                let expected_msg = &expected_program_error.cause;
                assert!(msg.find(expected_msg).is_some(), "Failed to find expected message `{expected_msg}` in `{msg}`");
            }
            Err(e) => {
                panic!("Unexpected error: `{e:?}`");
            }
        }
    }
} 

#[test]
fn test_command_invariants_pass_halt() {
    let contract_id = default_contract_id();
    let mut symbex = Symbex::from_contract(contract_id.clone(), r#"
        (define-map m uint uint)

        ;; (@clairvoyance
        ;;      (halt
        ;;          (result (ok false))
        ;;          (condition
        ;;              (and
        ;;                  (is-eq (mod (x uint) u2) u0)
        ;;                  (is-some (map-entry 'SP8H248H248H248H248H248H248H248H24ARTQ82.contract.m (x uint))))))
        ;;
        ;;      (halt
        ;;          (result (ok true))
        ;;          (condition
        ;;              (and
        ;;                  (is-eq (mod (x uint) u2) u0)
        ;;                  (is-none (map-entry 'SP8H248H248H248H248H248H248H248H24ARTQ82.contract.m (x uint)))))
        ;;          (map-write
        ;;              'SP8H248H248H248H248H248H248H248H24ARTQ82.contract.m
        ;;              (x uint)
        ;;              (x uint)))
        ;;
        ;;      (halt
        ;;          (result (err u0))
        ;;          (condition
        ;;              (not (is-eq (mod (x uint) u2) u0)))))
        ;;
        (define-public (set-if-odd (x uint))
            (if (is-eq (mod x u2) u0)
                (ok (map-insert m x x))
                (err u0)))
        "#,
    )
    .unwrap()
    .init()
    .unwrap();

    let termination_states = match symbex.eval_user_function("set-if-odd") {
        Ok(ts) => ts,
        Err(e) => {
            error!("symbex.eval_user_function: {e}");
            panic!()
        }
    };
    for t in termination_states.iter() {
        info!("{}", t.trace());
        info!("termination state: ==================================\n{}\n", &t.clone().rollup());
    }
}

#[test]
fn test_command_invariants_syntax_error() {
    let contract_id = default_contract_id();
    let mut symbex = Symbex::from_contract(contract_id.clone(), r#"
        (define-map m uint uint)

        ;; (@clairvoyance
        ;;      (halt
        ;;          (result (ok false))
        ;;          (invariant
        ;;              (and
        ;;                  (is-eq (mod (x uint) u2) u0)
        ;;                  (is-some (map-entry 'SP8H248H248H248H248H248H248H248H24ARTQ82.contract.m (x uint))))))
        ;;
        ;;      (halt
        ;;          (result (ok true))
        ;;          (invariant
        ;;              (and
        ;;                  (is-eq (mod (x uint) u2) u0)
        ;;                  (is-none (map-entry 'SP8H248H248H248H248H248H248H248H24ARTQ82.contract.m (x uint)))))
        ;;          (map-write
        ;;              'SP8H248H248H248H248H248H248H248H24ARTQ82.contract.m
        ;;              (x uint)
        ;;              (x uint)))
        ;;
        ;;      (halt
        ;;          (result (err u0))
        ;;          (invariant
        ;;              ;; oops
        ;;              (not (is-eq (mod (x uint)) u2)))))
        ;;
        (define-public (set-if-odd (x uint))
            (if (is-eq (mod x u2) u0)
                (ok (map-insert m x x))
                (err u0)))
        "#,
    )
    .unwrap()
    .init()
    .unwrap();

    match symbex.eval_user_function("set-if-odd") {
        Ok(termination_states) => {
            for t in termination_states.iter() {
                info!("{}", t.trace());
                info!("termination state: ==================================\n{}\n", &t.clone().rollup());
            }
            panic!("Did not encounter expected clairvoyance program syntax error");
        }
        Err(Error::Program(program_error)) => {
            info!("Program error:\n{program_error}\n");
            assert!(program_error.cause.find("has unexpected length 1 (expected at least 2)").is_some());
        }
        Err(e) => {
            error!("Unexpected error: {e:?}");
            panic!();
        }
    };
}

#[test]
fn test_command_invariants_unchecked_continuation() {
    let contract_id = default_contract_id();
    let mut symbex = Symbex::from_contract(contract_id.clone(), r#"
        (define-map m uint uint)

        ;; (@clairvoyance
        ;;      (invariant
        ;;          (ok false)
        ;;          (and
        ;;              (is-eq (mod (x uint) u2) u0)
        ;;              (is-some (map-entry 'SP8H248H248H248H248H248H248H248H24ARTQ82.contract.m (x uint)))))
        ;;
        ;;      (invariant
        ;;          (err u0)
        ;;          (not (is-eq (mod (x uint) u2) u0))))
        (define-public (set-if-odd (x uint))
            (if (is-eq (mod x u2) u0)
                (ok (map-insert m x x))
                (err u0)))
        "#,
    )
    .unwrap()
    .init()
    .unwrap();
    
    match symbex.eval_user_function("set-if-odd") {
        Ok(termination_states) => {
            for t in termination_states.iter() {
                info!("{}", t.trace());
                info!("termination state: ==================================\n{}\n", &t.clone().rollup());
            }
            panic!("Did not encounter expected clairvoyance proof failure error");
        }
        Err(Error::ProofFailure(proof_failure)) => {
            info!("proof failure:\n{proof_failure}\n");
            assert_eq!(proof_failure.unchecked_continuations.len(), 1);
            assert_eq!(proof_failure.unmatched_halting_conditions.len(), 0);
            assert_eq!(proof_failure.halting_conditions_failed.len(), 0);
        }
        Err(e) => {
            error!("Unexpected error: {e:?}");
            panic!();
        }
    }
}

#[test]
fn test_command_invariants_unmatched_invariant() {
    let contract_id = default_contract_id();
    let mut symbex = Symbex::from_contract(contract_id.clone(), r#"
        (define-map m uint uint)

        ;; (@clairvoyance
        ;;      (invariant
        ;;          (ok false)
        ;;          (and
        ;;              (is-eq (mod (x uint) u2) u0)
        ;;              (is-some (map-entry 'SP8H248H248H248H248H248H248H248H24ARTQ82.contract.m (x uint)))))
        ;;
        ;;      (invariant
        ;;          (ok true)
        ;;          (and
        ;;              (is-eq (mod (x uint) u2) u0)
        ;;              (is-none (map-entry 'SP8H248H248H248H248H248H248H248H24ARTQ82.contract.m (x uint)))))
        ;;
        ;;      (map-write
        ;;          (ok true)
        ;;          'SP8H248H248H248H248H248H248H248H24ARTQ82.contract.m
        ;;          (x uint)
        ;;          (x uint))
        ;;
        ;;      (invariant
        ;;          (err u1)
        ;;          (is-eq (x uint) u3))
        ;;
        ;;      (invariant
        ;;          (err u0)
        ;;          (not (is-eq (mod (x uint) u2) u0))))
        (define-public (set-if-odd (x uint))
            (if (is-eq (mod x u2) u0)
                (ok (map-insert m x x))
                (err u0)))
        "#,
    )
    .unwrap()
    .init()
    .unwrap();

    match symbex.eval_user_function("set-if-odd") {
        Ok(termination_states) => {
            for t in termination_states.iter() {
                info!("{}", t.trace());
                info!("termination state: ==================================\n{}\n", &t.clone().rollup());
            }
            panic!("Did not encounter expected clairvoyance proof failure error");
        }
        Err(Error::ProofFailure(proof_failure)) => {
            info!("proof failure:\n{proof_failure}\n");
            assert_eq!(proof_failure.unchecked_continuations.len(), 0);
            assert_eq!(proof_failure.unmatched_halting_conditions.len(), 1);
            assert_eq!(proof_failure.halting_conditions_failed.len(), 0);
        }
        Err(e) => {
            error!("Unexpected error: {e:?}");
            panic!();
        }
    }
}

#[test]
fn test_command_invariants_unproven_invariant() {
    let contract_id = default_contract_id();
    let mut symbex = Symbex::from_contract(contract_id.clone(), r#"
        (define-map m uint uint)

        ;; (@clairvoyance
        ;;      (invariant
        ;;          (ok false)
        ;;          (and
        ;;              (is-eq (mod (x uint) u2) u0)
        ;;              (is-some (map-entry 'SP8H248H248H248H248H248H248H248H24ARTQ82.contract.m (x uint)))))
        ;;
        ;;      (invariant
        ;;          (ok true)
        ;;          (and
        ;;              (is-eq (mod (x uint) u2) u0)
        ;;              (is-none (map-entry 'SP8H248H248H248H248H248H248H248H24ARTQ82.contract.m (x uint)))))
        ;;
        ;;      (invariant
        ;;          (ok true)
        ;;          (and
        ;;              (is-eq (x uint) u4)
        ;;              (is-none (map-entry 'SP8H248H248H248H248H248H248H248H24ARTQ82.contract.m (x uint)))))
        ;;
        ;;      (map-write
        ;;          (ok true)
        ;;          'SP8H248H248H248H248H248H248H248H24ARTQ82.contract.m
        ;;          (x uint)
        ;;          (x uint))
        ;;
        ;;      (invariant
        ;;          (err u0)
        ;;          (not (is-eq (mod (x uint) u2) u0))))
        (define-public (set-if-odd (x uint))
            (if (is-eq (mod x u2) u0)
                (ok (map-insert m x x))
                (err u0)))
        "#,
    )
    .unwrap()
    .init()
    .unwrap();

    match symbex.eval_user_function("set-if-odd") {
        Ok(termination_states) => {
            for t in termination_states.iter() {
                info!("{}", t.trace());
                info!("termination state: ==================================\n{}\n", &t.clone().rollup());
            }
            panic!("Did not encounter expected clairvoyance proof failure error");
        }
        Err(Error::ProofFailure(proof_failure)) => {
            info!("proof failure:\n{proof_failure}\n");
            assert_eq!(proof_failure.unchecked_continuations.len(), 0);
            assert_eq!(proof_failure.unmatched_halting_conditions.len(), 0);
            assert_eq!(proof_failure.halting_conditions_failed.len(), 1);
        }
        Err(e) => {
            error!("Unexpected error: {e:?}");
            panic!();
        }
    }
}

#[test]
fn test_command_define_formula() {
    let contract_id = default_contract_id();
    let mut symbex = Symbex::from_contract(contract_id.clone(), r#"
        (define-map m uint uint)

        ;; (@clairvoyance
        ;;      (define-symbol x-is-even (is-eq (mod (x uint) u2) u0))
        ;;      (define-symbol x-is-odd (not (x-is-even bool)))
        ;;      (define-symbol map-get-x (map-entry 'SP8H248H248H248H248H248H248H248H24ARTQ82.contract.m (x uint)))
        ;;
        ;;      (invariant
        ;;          (ok false)
        ;;          (and
        ;;              (x-is-even bool)
        ;;              (is-some (map-get-x (optional uint)))))
        ;;
        ;;      (invariant
        ;;          (ok true)
        ;;          (and
        ;;              (x-is-even bool)
        ;;              (is-none (map-get-x (optional uint)))))
        ;;
        ;;      (map-write
        ;;          (ok true)
        ;;          'SP8H248H248H248H248H248H248H248H24ARTQ82.contract.m
        ;;          (x uint)
        ;;          (x uint))
        ;;
        ;;      (invariant
        ;;          (err u0)
        ;;          (x-is-odd bool)))
        ;;
        (define-public (set-if-odd (x uint))
            (if (is-eq (mod x u2) u0)
                (ok (map-insert m x x))
                (err u0)))
        "#,
    )
    .unwrap()
    .init()
    .unwrap();

    let termination_states = match symbex.eval_user_function("set-if-odd") {
        Ok(ts) => ts,
        Err(e) => {
            error!("symbex.eval_user_function: {e}");
            panic!()
        }
    };
    for t in termination_states.iter() {
        info!("{}", t.trace());
        info!("termination state: ==================================\n{}\n", &t.clone().rollup());
    }
}
