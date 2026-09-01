;; Invariants that need an SMT solver, for `clairvoyance sym induct`.
;;
;;   clairvoyance sym induct SP000000000000000000002Q6VF78.smt examples/inductive-smt.clar
;;
;; Every invariant here is preserved, but the residual conditions are modular
;; or nonlinear, which is exactly where the algebraic simplifier gives out.
;; Run with `--no-smt` to see them all fall back to NOT PROVEN.

(define-data-var count uint u0)
(define-data-var n uint u0)
(define-data-var sq uint u0)

;; invariant: count is even
(define-read-only (invariant-count-even)
    (is-eq (mod (var-get count) u2) u0))

;; invariant: sq = n^2
(define-read-only (invariant-sq-eq-n-squared)
    (is-eq (var-get sq) (* (var-get n) (var-get n))))

;; Keeps count even. Needs modular arithmetic: (count + 2) mod 2 = count mod 2.
(define-public (add-two)
    (begin (var-set count (+ (var-get count) u2)) (ok true)))

;; Keeps sq = n^2. Needs the nonlinear identity (n+1)^2 = n^2 + 2n + 1.
(define-public (inc-n)
    (begin
        (var-set sq (+ (var-get sq) (+ (* u2 (var-get n)) u1)))
        (var-set n (+ (var-get n) u1))
        (ok true)))

;; Breaks count-even. No solver will prove this one -- it is a real violation,
;; and the printed condition is a genuine counterexample.
(define-public (add-one)
    (begin (var-set count (+ (var-get count) u1)) (ok true)))
