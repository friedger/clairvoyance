;; Which contract an argument to `contract-call?` is evaluated in.
;;
;;   clairvoyance sym induct SP000000000000000000002Q6VF78.ledger examples/caller-context.clar \
;;       --dep SP000000000000000000002Q6VF78.token:examples/caller-context-token.clar
;;
;; Clarity evaluates the arguments of a `contract-call?` in the caller, and
;; only then enters the callee: `current-contract` below is this ledger, not
;; the token. The invariant compares this ledger's book with the token's
;; balance *for this ledger*. `deposit` and `deposit-via-caller` credit that
;; balance and HOLD; `mint-to-token` credits the token's own balance instead
;; and must not.
;;
;; An engine that switched contracts before evaluating the arguments would
;; read the token's balance of *itself* in the invariant and in `deposit`,
;; consistently -- so it would still prove `deposit`, but it would also
;; prove `mint-to-token` and fail `deposit-via-caller`.

(define-data-var recorded uint u0)

(define-read-only (invariant-recorded-eq-balance)
    (is-eq (var-get recorded)
        (contract-call? 'SP000000000000000000002Q6VF78.token get-balance current-contract)))

(define-public (deposit (n uint))
    (begin
        (unwrap-panic (contract-call? 'SP000000000000000000002Q6VF78.token mint n current-contract))
        (var-set recorded (+ (var-get recorded) n))
        (ok true)))

(define-public (deposit-via-caller (n uint))
    (begin
        (unwrap-panic (contract-call? 'SP000000000000000000002Q6VF78.token mint-to-caller n))
        (var-set recorded (+ (var-get recorded) n))
        (ok true)))

(define-public (mint-to-token (n uint))
    (begin
        (unwrap-panic (contract-call? 'SP000000000000000000002Q6VF78.token mint n 'SP000000000000000000002Q6VF78.token))
        (var-set recorded (+ (var-get recorded) n))
        (ok true)))

(define-public (record-only (n uint))
    (begin (var-set recorded (+ (var-get recorded) n)) (ok true)))
