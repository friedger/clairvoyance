;; Clairvoyance command language -- reference grammar.
;;
;; This file is embedded into the binary (see `command.rs`,
;; `include_str!("./command.clar")`). It is documentation: the command
;; interpreter itself lives in Rust (`CommandContext::try_interpret` and
;; `Halt::from_symbolic_expressions`). Keeping the grammar here means a reader
;; who follows the `include_str!` finds the language written down.
;;
;; ---------------------------------------------------------------------------
;; Where commands live
;; ---------------------------------------------------------------------------
;;
;; A specification is written in a Clarity *comment* immediately above a
;; `define-public` / `define-read-only`, inside a `(@clairvoyance ...)` block.
;; The engine reads it off the function's pre-comments, runs the function
;; symbolically, and checks every terminating state against the commands. A
;; contract with no `(@clairvoyance ...)` block is simply explored, not checked.
;;
;;     ;; (@clairvoyance
;;     ;;     (invariant RESULT CONCLUSION)
;;     ;;     (halt ...))
;;     (define-public (f (x uint)) ...)
;;
;; ---------------------------------------------------------------------------
;; Symbolic operands
;; ---------------------------------------------------------------------------
;;
;; Operands are Clarity expressions over *symbols*. A free input is written
;; `(name type)`, e.g. `(x uint)`, `(who principal)`. Native Clarity operators
;; are written as usual: `(+ a b)`, `(is-eq a b)`, `(mod x u2)`, `(>= a b)`,
;; `(get field tuple)`, `(unwrap-panic opt)`, and so on.
;;
;; State is addressed by fully-qualified name:
;;     (var-get     'ADDR.contract.var-name)     ;; value written by this call
;;     (map-entry   'ADDR.contract.map-name KEY)     ;; = (map-get? map KEY)
;;
;; To name a var or map entry's value *on entry* -- before the call ran -- use
;; the `loaded-var` / `loaded-var-const` / `loaded-var-type` forms:
;;     (loaded-var  'ADDR.contract.var-name (v uint))  ;; the pre-call value
;;     (ft-get-balance 'ADDR.contract.token WHO)
;;     (stx-account WHO)
;;
;; ---------------------------------------------------------------------------
;; Commands
;; ---------------------------------------------------------------------------
;;
;; (test SYMOP)
;;     Parse-only: decode SYMOP and echo it. Useful for checking that an
;;     operand means what you think. The literal "force-failure!" fails on
;;     purpose, for testing the harness.
;;
;; (define-symbol NAME FORMULA)
;;     Bind NAME to FORMULA; later commands may use `(NAME type)` in place of
;;     FORMULA. Names are applied as rewrite rules, in order of definition.
;;
;; (invariant RESULT CONCLUSION)
;;     The short form. For every terminating state whose return value matches
;;     RESULT, the state's path predicate must imply CONCLUSION. A reachable
;;     state that no `invariant` (or `halt`) accounts for is reported as an
;;     `unchecked continuation`; an `invariant` that matches no state is an
;;     `unmatched halting condition`; a matched state whose predicate does not
;;     imply CONCLUSION is a `halting condition failed`.
;;
;; (halt ...)
;;     The long form: an invariant plus the exact state a matching terminating
;;     state must leave behind. Sub-directives:
;;
;;         (result SYMOP)                 required -- the return value to match
;;         (condition PREDICATE)          required -- implied by the path predicate
;;         (var-write   'ADDR.c.v VALUE)          -- this var must end at VALUE
;;         (map-write   'ADDR.c.m KEY VALUE)      -- this entry must end at VALUE
;;         (map-delete  'ADDR.c.m KEY)            -- this entry must be deleted
;;         (reachable-var-read   'ADDR.c.v)       -- with analyze-write-reachability
;;         (reachable-var-write  'ADDR.c.v)
;;         (reachable-map-read   'ADDR.c.m)
;;         (reachable-map-write  'ADDR.c.m)
;;         (early-return)                         -- state is an early (`try!`) return
;;         (panicking)                            -- state aborts (e.g. unwrap of none)
;;         (analyze-write-reachability)           -- also check the reachable-* sets
;;
;;     A `var-write` / `map-write` the function performs but no `halt` mentions
;;     is an `unchecked var/map write`; a `halt` that names a write the function
;;     does not perform is a `missing` one; a value that differs is `incorrect`.
;;
;; ---------------------------------------------------------------------------
;; Worked example
;; ---------------------------------------------------------------------------
;;
;;     ;; (@clairvoyance
;;     ;;     (invariant (err u0)
;;     ;;         (not (is-eq (mod (x uint) u2) u0)))
;;     ;;     (halt
;;     ;;         (result (ok true))
;;     ;;         (condition (is-eq (mod (x uint) u2) u0))
;;     ;;         (map-write 'SP8H248H248H248H248H248H248H248H24ARTQ82.contract.m
;;     ;;             (x uint) (x uint))))
;;     (define-map m uint uint)
;;     (define-public (set-if-even (x uint))
;;         (if (is-eq (mod x u2) u0)
;;             (ok (map-insert m x x))
;;             (err u0)))
