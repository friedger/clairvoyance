;; Regression: an invariant written as a bare `or`.
;;
;;   clairvoyance sym induct SP000000000000000000002Q6VF78.gap examples/or-invariant.clar
;;
;; `SymOp::or` used to build an `And` node, so every Clarity `(or a b)` -- in
;; invariants and in mutator guards alike -- was evaluated as `(and a b)`. On an
;; invariant that made the checked property stronger than the written one; in a
;; guard it hid the paths reachable under one disjunct alone. Both spellings
;; below must now decide all three mutators the same way:
;;
;;   HOLDS      clear-flag
;;   HOLDS      set-flag-zero-n
;;   NOT PROVEN break-it

(define-data-var flag bool false)
(define-data-var n uint u0)

;; invariant: the flag is only set while n is zero
(define-read-only (invariant-as-or)
    (or (not (var-get flag)) (is-eq (var-get n) u0)))

;; The same property, written as an `if`.
(define-read-only (invariant-as-if)
    (if (var-get flag) (is-eq (var-get n) u0) true))

;; preserves it: the flag goes down
(define-public (clear-flag) (begin (var-set flag false) (var-set n u1) (ok true)))
;; preserves it: n goes to zero
(define-public (set-flag-zero-n) (begin (var-set flag true) (var-set n u0) (ok true)))
;; breaks it
(define-public (break-it) (begin (var-set flag true) (var-set n u1) (ok true)))

;; A guard written as an `or`: reachable when either disjunct holds. Under the
;; old bug only the conjunction was explored, so the write to `n` on the
;; `flag`-only path was never seen and the invariant came back HOLDS.
(define-public (guarded-break)
    (begin
        (if (or (var-get flag) (is-eq (var-get n) u7))
            (var-set n u1)
            true)
        (ok true)))
