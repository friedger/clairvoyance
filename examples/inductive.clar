;; Inductive invariant demo for `clairvoyance sym induct`.
;;
;;   clairvoyance sym induct SP000000000000000000002Q6VF78.ledger examples/inductive.clar
;;
;; The invariant is `a == b`. `bump-both` keeps it; `bump-a` breaks it.

(define-data-var a uint u0)
(define-data-var b uint u0)

(define-read-only (invariant-a-eq-b)
    (is-eq (var-get a) (var-get b)))

;; preserves a == b
(define-public (bump-both)
    (begin
        (var-set a (+ (var-get a) u1))
        (var-set b (+ (var-get b) u1))
        (ok true)))

;; breaks a == b
(define-public (bump-a)
    (begin
        (var-set a (+ (var-get a) u1))
        (ok true)))

;; touches an unrelated var, so a == b is trivially preserved
(define-data-var c uint u0)
(define-public (bump-c)
    (begin
        (var-set c (+ (var-get c) u1))
        (ok true)))
