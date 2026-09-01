;; Cross-contract inductive invariant demo.
;;
;;   clairvoyance sym induct SP000000000000000000002Q6VF78.ledger examples/ledger.clar \
;;       --dep SP000000000000000000002Q6VF78.token:examples/token.clar
;;
;; The invariant spans two contracts: this ledger's `recorded` must equal the
;; token's `supply`, which it reads across the contract boundary. `deposit`
;; mints in the token AND records the same amount, so it preserves the
;; invariant (HOLDS); `record-only` records without minting and breaks it.

(define-data-var recorded uint u0)

(define-read-only (invariant-recorded-eq-supply)
    (is-eq (var-get recorded)
        (contract-call? 'SP000000000000000000002Q6VF78.token get-supply)))

(define-public (deposit (n uint))
    (begin
        (unwrap-panic (contract-call? 'SP000000000000000000002Q6VF78.token mint n))
        (var-set recorded (+ (var-get recorded) n))
        (ok true)))

(define-public (record-only (n uint))
    (begin (var-set recorded (+ (var-get recorded) n)) (ok true)))
