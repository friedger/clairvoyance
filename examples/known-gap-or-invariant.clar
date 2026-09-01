;; A known gap, kept as a runnable reproducer.
;;
;;   clairvoyance sym induct SP000000000000000000002Q6VF78.gap examples/known-gap-or-invariant.clar
;;
;; An invariant whose body is a bare `or` is not re-evaluated against what the
;; mutator wrote. Every mutator below reports NOT PROVEN with the *same*
;; residual -- the condition under which the invariant held on entry -- which
;; does not mention the mutator at all:
;;
;;   NOT PROVEN clear-flag       (fails when: (and (is-eq n u0) (not flag)))
;;   NOT PROVEN set-flag-zero-n  (fails when: (and (is-eq n u0) (not flag)))
;;   NOT PROVEN break-it         (fails when: (and (is-eq n u0) (not flag)))
;;
;; The first two preserve the invariant and ought to hold; the third breaks it
;; and ought to be reported as such. All three come back the same, so on an
;; `or` invariant the verdict carries no information about the mutator.
;;
;; It fails safe: the wrong answer is always NOT PROVEN, never HOLDS, so no
;; proof rests on it. A mutator that does not touch `flag` or `n` still holds
;; correctly, which is why an `or` invariant is not useless -- only its
;; not-proven results are.
;;
;; Writing the same property any other way works, which is the workaround:
;; `(if (var-get flag) (is-eq (var-get n) u0) true)` and
;; `(not (and (var-get flag) (not (is-eq (var-get n) u0))))` both decide all
;; three mutators correctly.

(define-data-var flag bool false)
(define-data-var n uint u0)

;; invariant: the flag is only set while n is zero
(define-read-only (invariant-as-or)
    (or (not (var-get flag)) (is-eq (var-get n) u0)))

;; The same property, written so the engine decides it.
(define-read-only (invariant-as-if)
    (if (var-get flag) (is-eq (var-get n) u0) true))

;; preserves it: the flag goes down
(define-public (clear-flag) (begin (var-set flag false) (var-set n u1) (ok true)))
;; preserves it: n goes to zero
(define-public (set-flag-zero-n) (begin (var-set flag true) (var-set n u0) (ok true)))
;; breaks it
(define-public (break-it) (begin (var-set flag true) (var-set n u1) (ok true)))
