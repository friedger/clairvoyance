;; A tiny token contract, used as a dependency in the cross-contract example.
;; See ledger.clar.
(define-data-var supply uint u0)
(define-read-only (get-supply) (var-get supply))
(define-public (mint (n uint))
    (begin (var-set supply (+ (var-get supply) n)) (ok true)))
