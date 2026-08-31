;; A tiny counter, specified for clairvoyance.
;;
;; Run:
;;   clairvoyance sym check SP000000000000000000002Q6VF78.counter examples/counter.clar --all
;;
;; `bump` is specified and should verify; `reset` has a deliberately wrong
;; specification so `check` reports it as VIOLATED; `peek` carries no spec.

(define-data-var count uint u0)

;; (@clairvoyance
;;     (halt
;;         (result (ok true))
;;         (condition true)
;;         (var-write 'SP000000000000000000002Q6VF78.counter.count
;;             (+ (loaded-var 'SP000000000000000000002Q6VF78.counter.count (count uint)) u1))))
(define-public (bump)
    (begin
        (var-set count (+ (var-get count) u1))
        (ok true)))

;; (@clairvoyance
;;     ;; WRONG on purpose: claims reset leaves the counter at u1, not u0.
;;     (halt
;;         (result (ok true))
;;         (condition true)
;;         (var-write 'SP000000000000000000002Q6VF78.counter.count u1)))
(define-public (reset)
    (begin
        (var-set count u0)
        (ok true)))

(define-read-only (peek)
    (var-get count))
