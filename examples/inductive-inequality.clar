;; Inequality invariant demo for `clairvoyance sym induct`.
;;
;;   clairvoyance sym induct SP000000000000000000002Q6VF78.leq examples/inductive-inequality.clar
;;
;; `bump-both` preserves lo <= hi: proving it needs to cancel the +u1 on both
;; sides of the inequality and see that a comparison and its complement cannot
;; both hold. The engine now does both, so this reports HOLDS.

(define-data-var lo uint u0)
(define-data-var hi uint u0)
;; invariant: lo <= hi
(define-read-only (invariant-lo-le-hi) (<= (var-get lo) (var-get hi)))
;; preserves lo <= hi  (lo<=hi  =>  lo+1 <= hi+1)
(define-public (bump-both)
    (begin (var-set lo (+ (var-get lo) u1)) (var-set hi (+ (var-get hi) u1)) (ok true)))
