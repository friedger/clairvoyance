;; A fungible-token invariant, for `clairvoyance sym induct`.
;;
;;   clairvoyance sym induct SP000000000000000000002Q6VF78.tok examples/inductive-token.clar
;;
;; The pattern behind every "the contract can cover what it owes" property: a
;; balance the contract does not control on one side, a book it does on the
;; other. `owe-funded` funds the debt as it takes it on and holds; `owe-unfunded`
;; does not, and the residual is a counterexample you can read.
;;
;; Nothing here assumes a starting balance -- an account the run has not written
;; reads as a free symbol -- so the proof is for any balance the chain might
;; have, not for a fresh contract.

(define-fungible-token tok)
(define-data-var owed uint u0)

;; invariant: the contract holds at least what it owes
(define-read-only (invariant-covers-owed)
    (>= (ft-get-balance tok current-contract) (var-get owed)))

;; Mints what it promises, so the margin is unchanged.
(define-public (owe-funded (n uint))
    (begin
        (try! (ft-mint? tok n current-contract))
        (var-set owed (+ (var-get owed) n))
        (ok true)))

;; Promises without funding, and eats into the margin.
(define-public (owe-unfunded (n uint))
    (begin (var-set owed (+ (var-get owed) n)) (ok true)))
