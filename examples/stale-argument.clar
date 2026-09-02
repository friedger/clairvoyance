;; Composition demo for `clairvoyance sym induct`: an argument is the
;; caller's value at the call site, not a formula re-read in the callee.
;;
;;   clairvoyance sym induct SP000000000000000000002Q6VF78.x examples/stale-argument.clar
;;
;; `cash-out` reads a member's balance, zeroes the record, and then hands the
;; amount it read to `debit`. The amount is the one read *before* the record
;; was zeroed, so `total + paid` stays equal to `supply`. An engine that
;; resolved the argument's map read against the caller's later write would
;; debit nothing, and report the invariant broken.

(define-map balances principal uint)
(define-data-var total uint u0)
(define-data-var paid uint u0)
(define-data-var supply uint u0)

(define-read-only (invariant-books-balance)
    (is-eq (+ (var-get total) (var-get paid)) (var-get supply)))

(define-private (debit (amount uint))
    (var-set total (- (var-get total) amount)))

(define-public (deposit (n uint))
    (begin
        (map-set balances tx-sender (+ (default-to u0 (map-get? balances tx-sender)) n))
        (var-set total (+ (var-get total) n))
        (var-set supply (+ (var-get supply) n))
        (ok true)))

(define-public (cash-out)
    (let ((amount (default-to u0 (map-get? balances tx-sender))))
        (map-set balances tx-sender u0)
        (debit amount)
        (var-set paid (+ (var-get paid) amount))
        (ok amount)))
