;; A token whose balances are keyed by principal, used as a dependency in the
;; caller-context example. See caller-context.clar.
(define-map balances principal uint)

(define-read-only (get-balance (who principal))
    (default-to u0 (map-get? balances who)))

(define-public (mint (n uint) (recipient principal))
    (begin
        (map-set balances recipient (+ (get-balance recipient) n))
        (ok true)))

;; Mints to whoever called this contract.
(define-public (mint-to-caller (n uint))
    (mint n contract-caller))
