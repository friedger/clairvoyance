// Copyright (C) 2026 Trust Machines
// 
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
// 
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
// 
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <http://www.gnu.org/licenses/>.

use std::collections::HashMap;
use std::collections::HashSet;

use clarity_types::types::{PrincipalData, StandardPrincipalData, QualifiedContractIdentifier, TraitIdentifier};
use clarity_types::ClarityName;

use crate::sym::Symbex;
use crate::sym::Continuation;
use crate::sym::Callgraph;
use crate::sym::FullName;
use crate::core::Error;
use crate::cli;

/// How many evaluation steps a single run may take before the engine reports
/// that it did not finish. Generous enough that an ordinary contract never
/// sees it, small enough that a blow-up stops in seconds rather than never.
const DEFAULT_STEP_BUDGET: Option<u64> = Some(2_000_000);

/// How long a single run may take before the engine reports that it did not
/// finish. Steps vary in cost by orders of magnitude, so this is the limit a
/// caller can actually predict; the step budget is the backstop.
const DEFAULT_TIME_BUDGET: Option<u64> = Some(60);

fn exec_user_function(
    contract_id: QualifiedContractIdentifier,
    src: &str,
    user_function: &str,
    deps: &[(QualifiedContractIdentifier, String)],
    concretized_traits: HashMap<FullName, HashMap<ClarityName, QualifiedContractIdentifier>>,
    default_concretized_traits: HashMap<TraitIdentifier, QualifiedContractIdentifier>,
    drop_early_returns: HashSet<FullName>,
    tx_sender: Option<StandardPrincipalData>,
    contract_caller: Option<PrincipalData>,
    tx_sponsor: Option<StandardPrincipalData>,
    contract_tx_sponsor: Option<StandardPrincipalData>,
    skip_functions: bool,
    skip_function_list: Vec<FullName>,
    // Abstract a called function that does no I/O as a symbol. It has no state
    // effects to lose, so this is safe even when the caller is being checked
    // for what it writes -- and it is most of what keeps exploration finite.
    skip_pure: bool,
    // Abstract a called function whose I/O is causally independent of the
    // current path. This one *can* hide a write, so anything reasoning about
    // state across a call has to leave it off.
    skip_causally_independent: bool,
    step_budget: Option<u64>,
    time_budget_secs: Option<u64>
) -> Result<Vec<Continuation>, Error> {
    let mut contracts : Vec<_> = deps 
        .iter()
        .map(|(contract_id, src)| {
            (
                contract_id.clone(),
                src.clone(),
                None
            )
        })
        .collect();

    contracts.push((contract_id, src.to_string(), contract_tx_sponsor));
    let target_contract_idx = contracts.len() - 1;

    let mut symbex = Symbex::from_contracts(contracts, target_contract_idx)?
        .with_tx_sender(tx_sender)
        .with_tx_sponsor(tx_sponsor)
        .with_contract_caller(contract_caller)
        .with_function_call_exploration(!skip_functions)
        .skip_pure(skip_pure)
        .skip_causally_independent(skip_causally_independent);
    symbex.step_budget = step_budget;
    if let Some(secs) = time_budget_secs {
        symbex.time_budget_secs = secs;
        symbex.deadline = Some(std::time::Instant::now() + std::time::Duration::from_secs(secs));
    }

    for (func_name, traits) in concretized_traits.iter() {
        for (var_name, contract_id) in traits.iter() {
            symbex = symbex
                .concretize_trait(func_name.clone(), var_name.clone(), contract_id.clone());
        }
    }

    for (trait_id, contract_id) in default_concretized_traits.iter() {
        symbex = symbex
            .default_trait(trait_id.clone(), contract_id.clone());
    }

    for func_name in drop_early_returns.into_iter() {
        symbex = symbex
            .drop_early_return(func_name);
    }

    for name in skip_function_list.into_iter() {
        symbex = symbex
            .with_skipped_function_call(name);
    }

    debug!("Symbolic execution begins on function '{user_function}'");
    symbex.eval_user_function(user_function)
}

fn cli_get_callgraph(
    contract_id: QualifiedContractIdentifier,
    src: &str,
    user_function: &str,
    deps: &[(QualifiedContractIdentifier, String)],
    concretized_traits: HashMap<FullName, HashMap<ClarityName, QualifiedContractIdentifier>>,
    default_concretized_traits: HashMap<TraitIdentifier, QualifiedContractIdentifier>,
) -> Result<(Callgraph, FullName), Error> {
    let mut contracts : Vec<_> = deps
        .iter()
        .map(|(contract_id, src)| {
            (
                contract_id.clone(),
                src.clone(),
                None
            )
        })
        .collect();

    contracts.push((contract_id.clone(), src.to_string(), None));
    let target_contract_idx = contracts.len() - 1;

    let mut symbex = Symbex::from_contracts(contracts, target_contract_idx)?;
    for (func_name, traits) in concretized_traits.iter() {
        for (var_name, contract_id) in traits.iter() {
            symbex = symbex
                .concretize_trait(func_name.clone(), var_name.clone(), contract_id.clone());
        }
    }

    for (trait_id, contract_id) in default_concretized_traits.iter() {
        symbex = symbex
            .default_trait(trait_id.clone(), contract_id.clone());
    }
    symbex = symbex.init()?;

    Ok((
        symbex.callgraph().clone(),
        FullName(contract_id, ClarityName::try_from(user_function).map_err(|_| Error::Invalid("Invalid function name {user_function}".into()))?)
    ))
}

/// Load the dependent contracts
/// format is `--dep CONTRACT_ID:/PATH/TO/CLARITY/CODE`
/// Contracts will be instantiated in the order given
fn load_deps(remaining_args: &mut Vec<String>) -> Result<Vec<(QualifiedContractIdentifier, String)>, (i32, String)> {
    let mut deps = vec![];
    loop {
        let contract_id_and_file = cli::consume_arg(remaining_args, &["--dep", "-c"], true);
        let (contract_id, src) = match contract_id_and_file {
            Ok(Some(contract_id_and_file)) => {
                let mut parts = contract_id_and_file.split(":");
                let Some(contract_id) = parts.next() else {
                    return Err((1, format!("dependency '{contract_id_and_file}' missing ':' delimiter")));
                };
                let Some(src_file) = parts.next() else {
                    return Err((1, format!("dependency '{contract_id_and_file}' missing source file")));
                };
                let Ok(contract_id) = QualifiedContractIdentifier::parse(&contract_id) else {
                    return Err((1, format!("Invalid dependency contract ID '{contract_id}'")));
                };
                let src = match cli::load_from_file_or_stdin(src_file) {
                    Ok(s) => match str::from_utf8(&s) {
                        Ok(src) => {
                            debug!("Loaded {}-byte source code from {}", src.len(), &src_file);
                            src.to_string()
                        }
                        Err(_) => {
                            return Err((1, format!("Dependency code in '{src_file}' is not UTF-8")));
                        }
                    }
                    Err(e) => {
                        return Err((1, format!("Failed to load source code from {src_file}: {e:?}")));
                    }
                };
                (contract_id, src)
            },
            Ok(None) => {
                break;
            }
            Err(e_str) => {
                return Err((1, e_str));
            }
        };
        debug!("Dependency: {contract_id}");
        deps.push((contract_id, src));
    }
    Ok(deps)
}

/// Load concretized traits
/// format is `--concretized-trait CONTRACT_ID.FUNCTION_NAME.VARIABLE_NAME:TRAIT_IMPL_CONTRACT_ID
fn load_concretized_traits(remaining_args: &mut Vec<String>) -> Result<HashMap<FullName, HashMap<ClarityName, QualifiedContractIdentifier>>, (i32, String)> {
    let mut concretized_traits : HashMap<FullName, HashMap<ClarityName, QualifiedContractIdentifier>> = HashMap::new();
    loop {
        let trait_binding = cli::consume_arg(remaining_args, &["--concretized-trait"], true);
        match trait_binding {
            Ok(Some(trait_binding)) => {
                let mut parts = trait_binding.split(":");
                let Some(fq_var_name) = parts.next() else {
                    return Err((1, format!("Failed to parse fully-qualified variable name from {trait_binding}")));
                };
                let Some(impl_contract_id) = parts.next() else {
                    return Err((1, format!("Failed to parse trait implementation contract from {trait_binding}")));
                };
                if parts.next().is_some() {
                    return Err((1, format!("Invalid value {trait_binding}: too many `:` separators")));
                };

                let Ok(impl_contract_id) = QualifiedContractIdentifier::parse(&impl_contract_id) else {
                    return Err((1, format!("Invalid contract ID {impl_contract_id}")));
                };

                // parse contract, function, variable
                let mut parts = fq_var_name.split(".");
                let Some(contract_address_str) = parts.next() else {
                    return Err((1, format!("Missing contract address in {fq_var_name}")));
                };
                let Some(contract_name_str) = parts.next() else {
                    return Err((1, format!("Missing contract name in {fq_var_name}")));
                };
                let Some(func_name_str) = parts.next() else {
                    return Err((1, format!("Missing function name in {fq_var_name}")));
                };
                let Some(var_name_str) = parts.next() else {
                    return Err((1, format!("Missing var name in {fq_var_name}")));
                };

                let Ok(contract_id) = QualifiedContractIdentifier::parse(&format!("{}.{}", contract_address_str, contract_name_str)) else {
                    return Err((1, format!("Could not parse `{contract_address_str}.{contract_name_str}`")));
                };
                let Ok(func_name) = ClarityName::try_from(func_name_str) else {
                    return Err((1, format!("Could not parse `{func_name_str}` -- invalid Clarity name")));
                };
                let fq_name = FullName(contract_id, func_name);
                let Ok(var_name) = ClarityName::try_from(var_name_str) else {
                    return Err((1, format!("Could not parse `{var_name_str}` -- invalid Clarity name")));
                };

                if let Some(traits) = concretized_traits.get_mut(&fq_name) {
                    traits.insert(var_name, impl_contract_id);
                }
                else {
                    let mut traits = HashMap::new();
                    traits.insert(var_name, impl_contract_id);
                    concretized_traits.insert(fq_name, traits);
                }
            }
            Ok(None) => {
                break;
            }
            Err(e_str) => {
                return Err((1, e_str));
            }
        }
    }
    Ok(concretized_traits)
}

/// Load default concretized traits
/// format is `--default-trait TRAIT_ID:TRAIT_IMPL_CONTRACT_ID`
fn load_default_concretized_traits(remaining_args: &mut Vec<String>) -> Result<HashMap<TraitIdentifier, QualifiedContractIdentifier>, (i32, String)> {
    let mut default_traits : HashMap<TraitIdentifier, QualifiedContractIdentifier> = HashMap::new();
    loop {
        let trait_binding = cli::consume_arg(remaining_args, &["--default-trait"], true);
        match trait_binding {
            Ok(Some(trait_binding)) => {
                let mut parts = trait_binding.split(":");
                let Some(trait_id_str) = parts.next() else {
                    return Err((1, format!("Failed to parse `{trait_binding}`")));
                };
                let Some(impl_contract_id) = parts.next() else {
                    return Err((1, format!("Missing contract name in `{trait_binding}`")));
                };

                let Ok(trait_id) = TraitIdentifier::parse_fully_qualified(trait_id_str) else {
                    return Err((1, format!("Failed to parse `{trait_id_str}`")));
                };
                let Ok(impl_contract_id) = QualifiedContractIdentifier::parse(&impl_contract_id) else {
                    return Err((1, format!("Invalid contract ID `{impl_contract_id}`")));
                };

                default_traits.insert(trait_id, impl_contract_id);
            }
            Ok(None) => {
                break;
            }
            Err(e_str) => {
                return Err((1, e_str));
            }
        }
    }
    Ok(default_traits)
}

/// Load the list of functions whose early-return continuations will not be explored
fn load_drop_early_returns(remaining_args: &mut Vec<String>) -> Result<HashSet<FullName>, (i32, String)> {
    let mut drop_early_returns : HashSet<FullName> = HashSet::new();
    loop {
        let early_return = cli::consume_arg(remaining_args, &["--drop-early-returns", "--drop-early-return"], true);
        match early_return {
            Ok(Some(func_name_s)) => {
                let Ok(func_name) = FullName::try_from(func_name_s.as_str()) else {
                    return Err((1, format!("Failed to parse full function name `{func_name_s}`")));
                };
                drop_early_returns.insert(func_name);
            }
            Ok(None) => {
                break;
            }
            Err(e_str) => {
                return Err((1, e_str));
            }
        }
    }
    Ok(drop_early_returns)
}

/// Load a standard principal from CLI args
fn load_standard_principal(remaining_args: &mut Vec<String>, arg_names: &[&str]) -> Result<Option<StandardPrincipalData>, (i32, String)> {
    let Some(principal) = load_principal(remaining_args, arg_names)? else {
        return Ok(None);
    };

    if let PrincipalData::Standard(data) = principal {
        Ok(Some(data))
    }
    else {
        Err((1, format!("Failed to parse principal {principal} as standard principal")))
    }
}

/// Load a contract principal from CLI args
fn load_principal(remaining_args: &mut Vec<String>, arg_names: &[&str]) -> Result<Option<PrincipalData>, (i32, String)> {
    let principal_res = cli::consume_arg(remaining_args, arg_names, true);
    let principal = match principal_res {
        Ok(Some(principal_s)) => {
            let Ok(principal) = PrincipalData::parse(&principal_s) else {
                return Err((1, format!("Failed to parse principal `{principal_s}`")));
            };
            Some(principal)
        }
        Ok(None) => {
            None
        }
        Err(e_str) => {
            return Err((1, e_str));
        }
    };
    debug!("Loaded {principal:?} from arguments {arg_names:?}");
    Ok(principal)
}

fn load_tx_sender(remaining_args: &mut Vec<String>) -> Result<Option<StandardPrincipalData>, (i32, String)> {
    load_standard_principal(remaining_args, &["--tx-sender"])
}

fn load_tx_sponsor(remaining_args: &mut Vec<String>) -> Result<Option<StandardPrincipalData>, (i32, String)> {
    load_standard_principal(remaining_args, &["--tx-sponsor"])
}

fn load_contract_caller(remaining_args: &mut Vec<String>) -> Result<Option<PrincipalData>, (i32, String)> {
    load_principal(remaining_args, &["--contract-caller"])
}

fn load_contract_tx_sponsor(remaining_args: &mut Vec<String>) -> Result<Option<StandardPrincipalData>, (i32, String)> {
    load_standard_principal(remaining_args, &["--contract-tx-sponsor"])
}

/// NOTE: This prints out continuations as they arrive.
fn cli_eval_user_function(argv: &[String]) -> (i32, String) {
    let mut remaining_args = argv.to_vec();

    // A path blow-up looks exactly like a hang from outside. A budget turns it
    // into a result: the run is reported as unfinished, never as holding.
    let time_budget = match cli::consume_arg(&mut remaining_args, &["--time-budget"], true) {
        Ok(Some(v)) => match v.parse::<u64>() {
            Ok(n) => Some(n),
            Err(_) => return (1, format!("--time-budget expects seconds, got '{v}'")),
        },
        Ok(None) => DEFAULT_TIME_BUDGET,
        Err(e) => return (1, e),
    };
    let step_budget = match cli::consume_arg(&mut remaining_args, &["--max-steps"], true) {
        Ok(Some(v)) => match v.parse::<u64>() {
            Ok(n) => Some(n),
            Err(_) => return (1, format!("--max-steps expects a number, got '{v}'")),
        },
        Ok(None) => DEFAULT_STEP_BUDGET,
        Err(e) => return (1, e),
    };
    let tx_sender = match load_tx_sender(&mut remaining_args) {
        Ok(p) => p,
        Err(x) => {
            return x;
        }
    };

    let tx_sponsor = match load_tx_sponsor(&mut remaining_args) {
        Ok(p) => p,
        Err(x) => {
            return x;
        }
    };
    
    let contract_caller = match load_contract_caller(&mut remaining_args) {
        Ok(p) => p,
        Err(x) => {
            return x;
        }
    };
    
    let contract_tx_sponsor = match load_contract_tx_sponsor(&mut remaining_args) {
        Ok(p) => p,
        Err(x) => {
            return x;
        }
    };

    let Ok(full_explore_opt) = cli::consume_arg(&mut remaining_args, &["--full", "-f"], false) else {
        return (1, format!("Could not parse --full"));
    };

    let Ok(no_explore_opt) = cli::consume_arg(&mut remaining_args, &["--no-explore-functions"], false) else {
        return (1, format!("Could not parse --no-explore-functions"));
    };

    let mut skip_functions = vec![];
    while let Ok(Some(fq_func_name_s)) = cli::consume_arg(&mut remaining_args, &["--skip-function"], true) {
        let Ok(name) = FullName::try_from(&fq_func_name_s) else {
            return (1, format!("Invalid function name '{fq_func_name_s}'"));
        };
        skip_functions.push(name);
    }

    let deps = match load_deps(&mut remaining_args) {
        Ok(deps) => deps,
        Err(x) => {
            return x;
        }
    };

    let concretized_traits = match load_concretized_traits(&mut remaining_args) {
        Ok(concretized_traits) => concretized_traits,
        Err(x) => {
            return x;
        }
    };

    let default_concretized_traits = match load_default_concretized_traits(&mut remaining_args) {
        Ok(concretized_traits) => concretized_traits,
        Err(x) => {
            return x;
        }
    };

    let drop_early_returns = match load_drop_early_returns(&mut remaining_args) {
        Ok(early_returns) => early_returns,
        Err(x) => {
            return x;
        }
    };
    
    let Some(contract_id_str) = argv.get(0) else {
        return (1, "Missing contract ID".into());
    };
    let Some(code_path_or_stdin) = argv.get(1) else {
        return (1, "Missing code".into());
    };
    let Some(user_function) = argv.get(2) else {
        return (1, "Missing user function".into())
    };
    let Ok(contract_id) = QualifiedContractIdentifier::parse(&contract_id_str) else {
        return (1, format!("Failed to parse contract ID {contract_id_str}"));
    };
    let src = match cli::load_from_file_or_stdin(code_path_or_stdin) {
        Ok(s) => match str::from_utf8(&s) {
            Ok(src) => {
                debug!("Loaded {}-byte source code from {}", src.len(), &code_path_or_stdin);
                src.to_string()
            }
            Err(_) => {
                return (1, format!("Code is not UTF-8"));
            }
        }
        Err(e) => {
            return (1, format!("Failed to load source code from {code_path_or_stdin}: {e:?}"));
        }
    };
    

    let continuations = match exec_user_function(
        contract_id,
        &src,
        user_function,
        &deps,
        concretized_traits,
        default_concretized_traits,
        drop_early_returns,
        tx_sender,
        contract_caller,
        tx_sponsor,
        contract_tx_sponsor,
        no_explore_opt.is_some(),
        skip_functions,
        !full_explore_opt.is_some(),
        !full_explore_opt.is_some(),
        step_budget,
        time_budget
    ) {
        Ok(c) => c,
        Err(e) => {
            let (code, msg) = explain_engine_error(user_function, &e);
            return (code, msg);
        }
    };

    let mut sbuf = "".to_string();
    for cont in continuations.into_iter() {
        sbuf.push_str(">>>>>>>>>>>>>>>>>>>> Terminating state:\n");
        sbuf.push_str(&format!("{}\n", &cont));
        let trace = cont.trace();
        sbuf.push_str(&format!("Stack trace:\n{}", &trace));
    }

    (0, sbuf)
}

/// Turn an engine error into a developer-facing message and exit code.
/// The two clairvoyance-specific errors get their own rendering; anything
/// else is a genuine engine/analysis failure.
///
///   3  a `(@clairvoyance ...)` specification was violated (the report says how)
///   4  a `(@clairvoyance ...)` specification could not be parsed
///   2  the engine could not evaluate the function (analysis error, or a term
///      it has no rule for)
fn explain_engine_error(user_function: &str, e: &Error) -> (i32, String) {
    match e {
        Error::ProofFailure(failures) => (
            3,
            format!("VIOLATED: `{user_function}` does not satisfy its (@clairvoyance ...) specification.\n\n{failures}"),
        ),
        Error::Program(program_error) => (
            4,
            format!("SPEC ERROR: the (@clairvoyance ...) block on `{user_function}` did not parse.\n\n{program_error}"),
        ),
        other => (
            2,
            format!("ERROR: could not evaluate `{user_function}`: {other}"),
        ),
    }
}

/// Condense an engine error into a single actionable line. The Clarity
/// analyzer's errors carry the whole offending AST, which is unreadable in a
/// per-function report -- and when a whole contract is missing, the same error
/// repeats once per function. The overwhelmingly common case, an unresolved
/// contract, becomes the thing to do about it.
fn brief_error(e: &Error) -> String {
    let text = format!("{e}");
    if let Some(rest) = text.split("NoSuchContract(\"").nth(1) {
        if let Some(name) = rest.split('"').next() {
            return format!("needs `{name}`; pass --dep {name}:PATH (a signature stub is enough)");
        }
    }
    // An analyzer error carries a human-readable diagnostic somewhere inside
    // the dumped AST; that sentence is the whole of what a reader needs.
    if let Some(rest) = text.split("message: \"").nth(1) {
        if let Some(message) = rest.split("\", spans").next().or_else(|| rest.split('"').next()) {
            return message.to_string();
        }
    }
    // Otherwise keep the first line, and only as much of it as is readable.
    let line = text.lines().next().unwrap_or("").trim();
    let line = line.split(" { ").next().unwrap_or(line);
    if line.len() > 200 { format!("{}...", &line[..200]) } else { line.to_string() }
}

/// Enumerate the public and read-only function names defined in `src`, in
/// source order. Parse-only, so it needs no dependency contracts loaded --
/// which is what lets `check --all` list the functions of a contract that
/// would not type-check in isolation.
fn list_functions(contract_id: &QualifiedContractIdentifier, src: &str) -> Result<Vec<String>, Error> {
    let ast = crate::core::ast::parse_ast(contract_id, src)?;
    let mut names = vec![];
    for expr in ast.expressions.iter() {
        let Some(list) = expr.match_list() else { continue; };
        let Some(head) = list.get(0).and_then(|e| e.match_atom()) else { continue; };
        if head.as_str() == "define-public" || head.as_str() == "define-read-only" {
            if let Some(sig) = list.get(1).and_then(|e| e.match_list()) {
                if let Some(fname) = sig.get(0).and_then(|e| e.match_atom()) {
                    names.push(fname.to_string());
                }
            }
        }
    }
    Ok(names)
}

fn summary_line(pass: usize, nospec: usize, fail: usize, specerr: usize, err: usize) -> String {
    format!("Summary: {pass} passed, {fail} violated, {specerr} spec-error, {err} error, {nospec} no-spec")
}

/// The parameter list of `fname`, each parameter rendered back to Clarity
/// source as `(name type)`. `None` if the function is not defined in `src`.
/// Parse-only, so no dependency contracts are needed.
fn function_params(
    contract_id: &QualifiedContractIdentifier,
    src: &str,
    fname: &str,
) -> Result<Option<Vec<String>>, Error> {
    let ast = crate::core::ast::parse_ast(contract_id, src)?;
    for expr in ast.expressions.iter() {
        let Some(list) = expr.match_list() else { continue; };
        let Some(head) = list.get(0).and_then(|e| e.match_atom()) else { continue; };
        if head.as_str() != "define-public" && head.as_str() != "define-read-only"
            && head.as_str() != "define-private"
        {
            continue;
        }
        let Some(sig) = list.get(1).and_then(|e| e.match_list()) else { continue; };
        let Some(name) = sig.get(0).and_then(|e| e.match_atom()) else { continue; };
        if name.as_str() != fname {
            continue;
        }
        let mut params = vec![];
        for p in sig.iter().skip(1) {
            // Each parameter is `(pname ptype...)`; render its type back to
            // source verbatim so exotic types (tuples, buffers) survive.
            if let Some(parts) = p.match_list() {
                let ty: Vec<String> = parts.iter().skip(1).map(|e| format!("{e}")).collect();
                params.push((format!("{}", parts.get(0).map(|e| format!("{e}")).unwrap_or_default()), ty.join(" ")));
            }
        }
        // Re-key parameters to fresh names to avoid collisions between the
        // mutator's and the invariant's parameters in the harness.
        let rendered: Vec<String> = params.into_iter().map(|(_n, ty)| ty).collect();
        return Ok(Some(rendered));
    }
    Ok(None)
}

// Sentinels the induction harness returns. Chosen large and specific so a
// mutator returning them by chance is vanishingly unlikely.
const INDUCT_PRE_SENTINEL: &str = "u340282366920938463463374607431768211454";
const INDUCT_POST_SENTINEL: &str = "u340282366920938463463374607431768211455";

/// Build a harness function, appended to the contract, that assumes the
/// invariant on entry, runs the mutator with fresh symbolic arguments, and
/// asserts the invariant still holds. If the invariant can fail afterwards,
/// the harness returns the post-sentinel error on that path.
///
///   (define-public (HARNESS <mut-params> <inv-params>)
///       (begin
///           (asserts! (INV <inv-args>) (err PRE))     ;; assume it held on entry
///           (unwrap-panic (MUT <mut-args>))           ;; run the mutator
///           (asserts! (INV <inv-args>) (err POST))    ;; it must still hold
///           (ok true)))
/// Print a line of the report as soon as it is known, rather than collecting
/// the whole report and showing it at the end.
fn print_now(line: &str) {
    print!("{line}");
    let _ = std::io::Write::flush(&mut std::io::stdout());
}

fn build_induction_harness(
    harness_name: &str,
    mutator: &str,
    mut_param_types: &[String],
    invariant: &str,
    inv_param_types: &[String],
) -> String {
    let mut sig = String::new();
    let mut mut_args = String::new();
    let mut inv_args = String::new();
    for (i, ty) in mut_param_types.iter().enumerate() {
        sig.push_str(&format!(" (m{i} {ty})"));
        mut_args.push_str(&format!(" m{i}"));
    }
    for (i, ty) in inv_param_types.iter().enumerate() {
        sig.push_str(&format!(" (i{i} {ty})"));
        inv_args.push_str(&format!(" i{i}"));
    }
    format!(
        "\n(define-public ({harness_name}{sig})\n  \
           (begin\n    \
             (asserts! ({invariant}{inv_args}) (err {pre}))\n    \
             (unwrap-panic ({mutator}{mut_args}))\n    \
             (asserts! ({invariant}{inv_args}) (err {post}))\n    \
             (ok true)))\n",
        pre = INDUCT_PRE_SENTINEL,
        post = INDUCT_POST_SENTINEL,
    )
}

/// `sym check` -- verify a function (or every function) against the
/// (@clairvoyance ...) specification in its doc comment, and report a
/// developer-facing PASS / VIOLATED / SPEC ERROR verdict with a meaningful
/// exit code. This is the ergonomic front door onto the same engine that
/// `exec-func` drives; the difference is the output and the exit code.
fn cli_check(argv: &[String]) -> (i32, String) {
    let mut remaining_args = argv.to_vec();

    // A path blow-up looks exactly like a hang from outside. A budget turns it
    // into a result: the run is reported as unfinished, never as holding.
    let time_budget = match cli::consume_arg(&mut remaining_args, &["--time-budget"], true) {
        Ok(Some(v)) => match v.parse::<u64>() {
            Ok(n) => Some(n),
            Err(_) => return (1, format!("--time-budget expects seconds, got '{v}'")),
        },
        Ok(None) => DEFAULT_TIME_BUDGET,
        Err(e) => return (1, e),
    };
    let step_budget = match cli::consume_arg(&mut remaining_args, &["--max-steps"], true) {
        Ok(Some(v)) => match v.parse::<u64>() {
            Ok(n) => Some(n),
            Err(_) => return (1, format!("--max-steps expects a number, got '{v}'")),
        },
        Ok(None) => DEFAULT_STEP_BUDGET,
        Err(e) => return (1, e),
    };
    let tx_sender = match load_tx_sender(&mut remaining_args) { Ok(p) => p, Err(x) => return x };
    let tx_sponsor = match load_tx_sponsor(&mut remaining_args) { Ok(p) => p, Err(x) => return x };
    let contract_caller = match load_contract_caller(&mut remaining_args) { Ok(p) => p, Err(x) => return x };
    let contract_tx_sponsor = match load_contract_tx_sponsor(&mut remaining_args) { Ok(p) => p, Err(x) => return x };

    let Ok(full_explore_opt) = cli::consume_arg(&mut remaining_args, &["--full", "-f"], false) else {
        return (1, "Could not parse --full".into());
    };
    let Ok(no_explore_opt) = cli::consume_arg(&mut remaining_args, &["--no-explore-functions"], false) else {
        return (1, "Could not parse --no-explore-functions".into());
    };
    let Ok(all_opt) = cli::consume_arg(&mut remaining_args, &["--all"], false) else {
        return (1, "Could not parse --all".into());
    };

    let mut skip_functions = vec![];
    while let Ok(Some(fq_func_name_s)) = cli::consume_arg(&mut remaining_args, &["--skip-function"], true) {
        let Ok(name) = FullName::try_from(&fq_func_name_s) else {
            return (1, format!("Invalid function name '{fq_func_name_s}'"));
        };
        skip_functions.push(name);
    }

    let deps = match load_deps(&mut remaining_args) { Ok(deps) => deps, Err(x) => return x };
    let concretized_traits = match load_concretized_traits(&mut remaining_args) { Ok(t) => t, Err(x) => return x };
    let default_concretized_traits = match load_default_concretized_traits(&mut remaining_args) { Ok(t) => t, Err(x) => return x };
    let drop_early_returns = match load_drop_early_returns(&mut remaining_args) { Ok(r) => r, Err(x) => return x };

    let Some(contract_id_str) = argv.get(0) else {
        return (1, "Missing contract ID".into());
    };
    let Some(code_path_or_stdin) = argv.get(1) else {
        return (1, "Missing code".into());
    };
    let Ok(contract_id) = QualifiedContractIdentifier::parse(contract_id_str) else {
        return (1, format!("Failed to parse contract ID {contract_id_str}"));
    };
    let src = match cli::load_from_file_or_stdin(code_path_or_stdin) {
        Ok(s) => match String::from_utf8(s) {
            Ok(src) => src,
            Err(_) => return (1, "Code is not UTF-8".into()),
        },
        Err(e) => return (1, format!("Failed to load source code from {code_path_or_stdin}: {e:?}")),
    };

    // Which functions to check: the named one, or (with --all, or when no
    // function is named) every public and read-only function in the contract.
    let explicit_fn = argv.get(2).filter(|s| !s.starts_with('-')).cloned();
    let functions = if all_opt.is_some() || explicit_fn.is_none() {
        match list_functions(&contract_id, &src) {
            Ok(fns) if !fns.is_empty() => fns,
            Ok(_) => return (1, "No public or read-only functions found to check".into()),
            Err(e) => return (2, format!("Could not parse contract to list its functions: {e}")),
        }
    } else {
        vec![explicit_fn.expect("infallible: is_some checked")]
    };

    let multi = functions.len() > 1;
    let has_spec = src.contains("@clairvoyance");
    let mut report = String::new();
    let (mut n_pass, mut n_nospec, mut n_fail, mut n_specerr, mut n_err) = (0, 0, 0, 0, 0);

    for func in functions.iter() {
        let res = exec_user_function(
            contract_id.clone(),
            &src,
            func,
            &deps,
            concretized_traits.clone(),
            default_concretized_traits.clone(),
            drop_early_returns.clone(),
            tx_sender.clone(),
            contract_caller.clone(),
            tx_sponsor.clone(),
            contract_tx_sponsor.clone(),
            no_explore_opt.is_some(),
            skip_functions.clone(),
            !full_explore_opt.is_some(),
            !full_explore_opt.is_some(),
            step_budget,
            time_budget,
        );
        match res {
            Ok(conts) => {
                if has_spec {
                    n_pass += 1;
                    report.push_str(&format!("  PASS      {func}  ({} state(s))\n", conts.len()));
                } else {
                    n_nospec += 1;
                    report.push_str(&format!("  NO SPEC   {func}  ({} state(s) explored)\n", conts.len()));
                }
            }
            Err(Error::ProofFailure(failures)) => {
                n_fail += 1;
                if multi {
                    report.push_str(&format!("  VIOLATED  {func}\n"));
                } else {
                    report.push_str(&format!(
                        "VIOLATED: `{func}` does not satisfy its (@clairvoyance ...) specification.\n\n{failures}\n"
                    ));
                }
            }
            Err(Error::Program(program_error)) => {
                n_specerr += 1;
                if multi {
                    report.push_str(&format!("  SPEC ERR  {func}\n"));
                } else {
                    report.push_str(&format!("SPEC ERROR on `{func}`:\n\n{program_error}\n"));
                }
            }
            Err(other) => {
                n_err += 1;
                if multi {
                    report.push_str(&format!("  ERROR     {func}  ({other})\n"));
                } else {
                    report.push_str(&format!("ERROR evaluating `{func}`: {other}\n"));
                }
            }
        }
    }

    if multi {
        let hint = if n_fail > 0 || n_specerr > 0 {
            "\n\nRe-run `sym check <CONTRACT_ID> <CODE> <FUNCTION>` on a failing function for the full report."
        } else {
            ""
        };
        report = format!(
            "Checked {} function(s) in {}:\n\n{}\n{}{}\n",
            functions.len(),
            contract_id,
            report,
            summary_line(n_pass, n_nospec, n_fail, n_specerr, n_err),
            hint,
        );
    }

    let code = if n_fail > 0 { 3 } else if n_specerr > 0 { 4 } else if n_err > 0 { 2 } else { 0 };
    (code, report)
}

/// Public and read-only function names of a contract, in source order.
fn functions_by_kind(
    contract_id: &QualifiedContractIdentifier,
    src: &str,
) -> Result<(Vec<String>, Vec<String>), Error> {
    let ast = crate::core::ast::parse_ast(contract_id, src)?;
    let (mut publics, mut readonlys) = (vec![], vec![]);
    for expr in ast.expressions.iter() {
        let Some(list) = expr.match_list() else { continue; };
        let Some(head) = list.get(0).and_then(|e| e.match_atom()) else { continue; };
        let Some(sig) = list.get(1).and_then(|e| e.match_list()) else { continue; };
        let Some(name) = sig.get(0).and_then(|e| e.match_atom()) else { continue; };
        match head.as_str() {
            "define-public" => publics.push(name.to_string()),
            "define-read-only" => readonlys.push(name.to_string()),
            _ => {}
        }
    }
    Ok((publics, readonlys))
}

/// `sym induct` -- inductive invariant checking.
///
/// For each (invariant I, mutator M) pair, synthesize a harness that assumes I
/// on entry, runs M over fresh symbolic arguments, and asserts I still holds;
/// symbolically execute it, and report whether I is preserved:
///
///   HOLDS       the engine proved I holds after M on every path
///   VIOLATED    I fails after M unconditionally (a definite counterexample)
///   NOT PROVEN  the engine could not rule out a path where I fails after M;
///               the residual condition is shown. This is either a real
///               conditional violation or a limit of the simplifier (there is
///               no SMT backend), and the condition tells you which to suspect.
///
/// With no `--invariant`, every read-only named `invariant-*` is used; with no
/// `--mutator`, every public function is used.
fn cli_induct(argv: &[String]) -> (i32, String) {
    let mut remaining_args = argv.to_vec();

    // A path blow-up looks exactly like a hang from outside. A budget turns it
    // into a result: the run is reported as unfinished, never as holding.
    let time_budget = match cli::consume_arg(&mut remaining_args, &["--time-budget"], true) {
        Ok(Some(v)) => match v.parse::<u64>() {
            Ok(n) => Some(n),
            Err(_) => return (1, format!("--time-budget expects seconds, got '{v}'")),
        },
        Ok(None) => DEFAULT_TIME_BUDGET,
        Err(e) => return (1, e),
    };
    let step_budget = match cli::consume_arg(&mut remaining_args, &["--max-steps"], true) {
        Ok(Some(v)) => match v.parse::<u64>() {
            Ok(n) => Some(n),
            Err(_) => return (1, format!("--max-steps expects a number, got '{v}'")),
        },
        Ok(None) => DEFAULT_STEP_BUDGET,
        Err(e) => return (1, e),
    };
    let tx_sender = match load_tx_sender(&mut remaining_args) { Ok(p) => p, Err(x) => return x };
    let tx_sponsor = match load_tx_sponsor(&mut remaining_args) { Ok(p) => p, Err(x) => return x };
    let contract_caller = match load_contract_caller(&mut remaining_args) { Ok(p) => p, Err(x) => return x };
    let contract_tx_sponsor = match load_contract_tx_sponsor(&mut remaining_args) { Ok(p) => p, Err(x) => return x };

    let mut invariants = vec![];
    while let Ok(Some(name)) = cli::consume_arg(&mut remaining_args, &["--invariant"], true) {
        invariants.push(name);
    }
    let mut mutators = vec![];
    while let Ok(Some(name)) = cli::consume_arg(&mut remaining_args, &["--mutator"], true) {
        mutators.push(name);
    }

    // An SMT solver decides the paths the simplifier cannot. It may only ever
    // turn NOT PROVEN into HOLDS -- see the `smt` module on why `Unsat` is the
    // only trustworthy answer -- so running without one is always safe.
    let Ok(no_smt) = cli::consume_arg(&mut remaining_args, &["--no-smt"], false) else {
        return (1, "Could not parse --no-smt".into());
    };
    let solver_override = match cli::consume_arg(&mut remaining_args, &["--solver"], true) {
        Ok(v) => v,
        Err(e) => return (1, e),
    };
    let solver = if no_smt.is_some() {
        None
    } else if let Some(program) = solver_override {
        // An explicit --solver that does not run is an error, not a fallback:
        // silently reporting the weaker simplifier-only answers under a header
        // that names the solver would be the wrong kind of quiet.
        match crate::smt::solver_at(&program) {
            Some(s) => Some(s),
            None => {
                return (
                    1,
                    format!(
                        "Could not run the SMT solver `{program}`. Pass a path to an \
                         SMT-LIB 2 solver, or --no-smt to check without one."
                    ),
                );
            }
        }
    } else {
        crate::smt::find_solver()
    };

    let deps = match load_deps(&mut remaining_args) { Ok(deps) => deps, Err(x) => return x };
    let concretized_traits = match load_concretized_traits(&mut remaining_args) { Ok(t) => t, Err(x) => return x };
    let default_concretized_traits = match load_default_concretized_traits(&mut remaining_args) { Ok(t) => t, Err(x) => return x };

    let Some(contract_id_str) = argv.get(0) else { return (1, "Missing contract ID".into()); };
    let Some(code_path_or_stdin) = argv.get(1) else { return (1, "Missing code".into()); };
    let Ok(contract_id) = QualifiedContractIdentifier::parse(contract_id_str) else {
        return (1, format!("Failed to parse contract ID {contract_id_str}"));
    };
    let src = match cli::load_from_file_or_stdin(code_path_or_stdin) {
        Ok(s) => match String::from_utf8(s) { Ok(src) => src, Err(_) => return (1, "Code is not UTF-8".into()) },
        Err(e) => return (1, format!("Failed to load source code from {code_path_or_stdin}: {e:?}")),
    };

    let (publics, readonlys) = match functions_by_kind(&contract_id, &src) {
        Ok(x) => x,
        Err(e) => return (2, format!("Could not parse contract: {e}")),
    };
    if invariants.is_empty() {
        invariants = readonlys.iter().filter(|n| n.starts_with("invariant-")).cloned().collect();
    }
    if mutators.is_empty() {
        mutators = publics.clone();
    }
    if invariants.is_empty() {
        return (1, "No invariants to check. Name one with --invariant, or define read-only invariant-* functions.".into());
    }
    if mutators.is_empty() {
        return (1, "No mutators to check. Name one with --mutator.".into());
    }

    let post_err = format!("(err {})", INDUCT_POST_SENTINEL);
    let (mut n_holds, mut n_violated, mut n_unproven, mut n_skipped) = (0, 0, 0, 0);
    let mut n_holds_smt = 0;
    // Skip reasons already spelled out in full, so a missing dependency does
    // not print its explanation once per (invariant, mutator) pair.
    let mut skip_reasons = std::collections::HashSet::new();

    // A run over a large contract takes minutes, and a report that appears only
    // at the end is indistinguishable from a hang. Each verdict is printed as
    // it lands; the returned string is just the summary.
    let engine = match solver.as_ref() {
        Some(s) => format!("simplifier + {}", s.program),
        None if no_smt.is_some() => "simplifier only (--no-smt)".to_string(),
        None => "simplifier only (no SMT solver found; install z3, or pass --solver PATH)"
            .to_string(),
    };
    println!(
        "Inductive invariant check for {contract_id}\n\
         (does each mutator preserve each invariant?)\n\
         decided by: {engine}"
    );
    let _ = std::io::Write::flush(&mut std::io::stdout());

    for inv in invariants.iter() {
        let inv_params = match function_params(&contract_id, &src, inv) {
            Ok(Some(p)) => p,
            _ => { print_now(&format!("\n{inv}\n  (invariant not found; skipping)\n")); continue; }
        };
        print_now(&format!("\n{inv}\n"));
        for mutator in mutators.iter() {
            let mut_params = match function_params(&contract_id, &src, mutator) {
                Ok(Some(p)) => p,
                _ => { print_now(&format!("  SKIP       {mutator}  (not found)\n")); n_skipped += 1; continue; }
            };
            let harness_name = format!("clv-induct-{mutator}-{inv}");
            let harness = build_induction_harness(&harness_name, mutator, &mut_params, inv, &inv_params);
            let src2 = format!("{src}{harness}");

            let res = exec_user_function(
                contract_id.clone(), &src2, &harness_name, &deps,
                concretized_traits.clone(), default_concretized_traits.clone(),
                std::collections::HashSet::new(),
                tx_sender.clone(), contract_caller.clone(), tx_sponsor.clone(), contract_tx_sponsor.clone(),
                false, vec![],
                // Pure calls stay abstracted -- they cannot carry state
                // between the mutator and the invariant -- but everything that
                // touches state is inlined, which is what lets a value the
                // mutator writes reach the invariant that reads it.
                true,
                false,
                step_budget,
                time_budget,
            );
            match res {
                Ok(conts) => {
                    let violations: Vec<_> = conts.iter()
                        .filter(|c| format!("{}", c.final_formula) == post_err)
                        .collect();
                    if violations.is_empty() {
                        n_holds += 1;
                        print_now(&format!("  HOLDS      {mutator}\n"));
                    } else if violations.iter().any(|c| format!("{}", c.predicate).trim() == "true") {
                        n_violated += 1;
                        print_now(&format!("  VIOLATED   {mutator}  (unconditionally)\n"));
                    } else if solver.as_ref().is_some_and(|s| {
                        // The simplifier could not rule these paths out. Ask a
                        // solver. Only `Unsat` counts: the translation is an
                        // over-approximation, so `Sat` proves nothing (see the
                        // `smt` module). Every violating path must be
                        // infeasible for the invariant to be preserved.
                        violations.iter().all(|c| {
                            crate::smt::predicate_is_unsat(&c.predicate, s) == crate::smt::Answer::Unsat
                        })
                    }) {
                        n_holds += 1;
                        n_holds_smt += 1;
                        print_now(&format!("  HOLDS      {mutator}  (by solver)\n"));
                    } else {
                        n_unproven += 1;
                        let cond = format!("{}", violations[0].predicate);
                        let cond = cond.split_whitespace().collect::<Vec<_>>().join(" ");
                        let cond = if cond.len() > 160 { format!("{}...", &cond[..160]) } else { cond };
                        print_now(&format!("  NOT PROVEN {mutator}  (fails when: {cond})\n"));
                    }
                }
                Err(Error::TimedOut(secs)) => {
                    n_unproven += 1;
                    print_now(&format!(
                        "  UNFINISHED {mutator}  (gave up after {secs}s; raise --time-budget)\n"
                    ));
                }
                Err(Error::Budget(steps)) => {
                    // Tried and did not finish, which is a different thing
                    // from not having tried: it counts against the proof.
                    n_unproven += 1;
                    print_now(&format!(
                        "  UNFINISHED {mutator}  (gave up after {steps} steps; raise --max-steps)\n"
                    ));
                }
                Err(e) => {
                    n_skipped += 1;
                    let why = brief_error(&e);
                    // The same missing dependency skips every pair, so say it
                    // once and just name the rest.
                    if skip_reasons.insert(why.clone()) {
                        print_now(&format!("  SKIP       {mutator}  ({why})\n"));
                    } else {
                        print_now(&format!("  SKIP       {mutator}\n"));
                    }
                }
            }
        }
    }

    let footer = format!(
        "\nSummary: {n_holds} holds ({n_holds_smt} by solver), {n_violated} violated, \
         {n_unproven} not-proven, {n_skipped} skipped\n"
    );
    let code = if n_violated > 0 { 3 } else if n_unproven > 0 { 5 } else { 0 };
    (code, footer)
}

fn cli_reachability_graph(argv: &[String]) -> (i32, String) {
    let Some(contract_id_str) = argv.get(0) else {
        return (1, "Missing contract ID".into());
    };
    let Some(code_path_or_stdin) = argv.get(1) else {
        return (1, "Missing code".into());
    };
    let Some(user_function) = argv.get(2) else {
        return (1, "Missing user function".into())
    };
    let Ok(contract_id) = QualifiedContractIdentifier::parse(&contract_id_str) else {
        return (1, format!("Failed to parse contract ID {contract_id_str}"));
    };
    let src = match cli::load_from_file_or_stdin(code_path_or_stdin) {
        Ok(s) => match str::from_utf8(&s) {
            Ok(src) => {
                debug!("Loaded {}-byte source code from {}", src.len(), &code_path_or_stdin);
                src.to_string()
            }
            Err(_) => {
                return (1, format!("Code is not UTF-8"));
            }
        }
        Err(e) => {
            return (1, format!("Failed to load source code from {code_path_or_stdin}: {e:?}"));
        }
    };
    
    let mut remaining_args = argv.to_vec();

    let deps = match load_deps(&mut remaining_args) {
        Ok(deps) => deps,
        Err(x) => {
            return x;
        }
    };

    let concretized_traits = match load_concretized_traits(&mut remaining_args) {
        Ok(concretized_traits) => concretized_traits,
        Err(x) => {
            return x;
        }
    };

    let default_concretized_traits = match load_default_concretized_traits(&mut remaining_args) {
        Ok(concretized_traits) => concretized_traits,
        Err(x) => {
            return x;
        }
    };

    let (callgraph, user_function) = match cli_get_callgraph(contract_id, &src, user_function, &deps, concretized_traits, default_concretized_traits) {
        Ok((cg, uf)) => (cg, uf),
        Err(e) => {
            return (2, format!("Failed to build callgraph for function {user_function}: {e:?}"));
        }
    };

    let Some(view) = callgraph.view(&user_function) else {
        return (1, format!("No such function: {user_function}"));
    };
    return (0, view.to_string());
}

pub fn run_cli_sym(argv: &mut Vec<String>) -> (i32, String) {
    if argv.len() == 0 {
        return (1, "Missing subcommand".into());
    }

    let subcommand = argv.remove(0);
    match subcommand.as_str() {
        "exec-func" => {
            cli_eval_user_function(&argv)
        }
        "check" => {
            cli_check(&argv)
        }
        "induct" => {
            cli_induct(&argv)
        }
        "reachable" => {
            cli_reachability_graph(&argv)
        }
        "help" | "-h" | "--help" => {
            (0, sym_help())
        }
        _ => {
            (1, format!("Unrecognized `sym` command '{subcommand}'. Try `sym help` for details."))
        }
    }
}

/// Usage text for the `sym` subcommands.
fn sym_help() -> String {
    "clairvoyance sym -- symbolically execute and verify Clarity functions\n\
     \n\
     USAGE:\n\
     \x20 clairvoyance sym check     CONTRACT_ID CODE [FUNCTION] [options]\n\
     \x20 clairvoyance sym induct    CONTRACT_ID CODE [options]\n\
     \x20 clairvoyance sym exec-func CONTRACT_ID CODE FUNCTION  [options]\n\
     \x20 clairvoyance sym reachable CONTRACT_ID CODE FUNCTION  [options]\n\
     \x20 clairvoyance sym help\n\
     \n\
     COMMANDS:\n\
     \x20 check      Verify a function against the (@clairvoyance ...) specification\n\
     \x20            written in its doc comment, and report PASS / VIOLATED / SPEC ERROR.\n\
     \x20            Omit FUNCTION (or pass --all) to check every public and read-only\n\
     \x20            function in the contract and print a summary. Exit code is 0 only\n\
     \x20            when everything checked passes.\n\
     \x20 induct     Inductive invariant checking. For each (invariant, mutator) pair,\n\
     \x20            assume the invariant, run the mutator, and check it still holds.\n\
     \x20            Reports HOLDS / VIOLATED / NOT PROVEN. With no --invariant, every\n\
     \x20            read-only named invariant-* is used; with no --mutator, every public\n\
     \x20            function. Reports UNFINISHED for a pair that ran past --max-steps.\n\
     \x20            Options: --invariant NAME, --mutator NAME (both repeatable),\n\
     \x20            --solver PATH, --no-smt, --time-budget SECONDS, --max-steps N.\n\
     \x20 exec-func  Symbolically execute FUNCTION and print every terminating state\n\
     \x20            (path predicate, return value, and state writes). Also enforces any\n\
     \x20            (@clairvoyance ...) spec, like `check`, but prints the raw states.\n\
     \x20 reachable  Print the call graph reachable from FUNCTION (which vars/maps it may\n\
     \x20            read and write, transitively).\n\
     \n\
     ARGUMENTS:\n\
     \x20 CONTRACT_ID  Qualified contract id, e.g. SP000...000.my-contract\n\
     \x20 CODE         Path to the .clar source, or `-` to read from stdin\n\
     \x20 FUNCTION     Name of the function to analyze\n\
     \n\
     OPTIONS:\n\
     \x20 --all                        (check only) check every public/read-only function\n\
     \x20 --dep CONTRACT_ID:PATH       Load a dependency contract (repeatable). Instantiated\n\
     \x20                              in the order given, before the target.\n\
     \x20 --concretized-trait C.f.v:IMPL   Bind trait variable `v` of function `f` in contract\n\
     \x20                              `C` to a concrete implementation contract.\n\
     \x20 --default-trait TRAIT:IMPL   Bind a trait to a default implementation everywhere.\n\
     \x20 --tx-sender PRINCIPAL        Set tx-sender (default: a fresh symbol).\n\
     \x20 --contract-caller PRINCIPAL  Set contract-caller.\n\
     \x20 --tx-sponsor PRINCIPAL       Set the transaction sponsor.\n\
     \x20 --skip-function C.c.f        Do not descend into this function; treat it as opaque.\n\
     \x20 --no-explore-functions       Do not descend into any called function.\n\
     \x20 --full, -f                   Explore all paths, including pure/causally-independent\n\
     \x20                              ones normally pruned.\n\
     \x20 --solver PATH                (induct only) SMT solver to discharge the residual\n\
     \x20                              conditions the simplifier cannot. Default: $CLAIRVOYANCE_SMT,\n\
     \x20                              else z3 or cvc5 if one is on PATH.\n\
     \x20 --no-smt                     (induct only) do not use a solver, even if one is\n\
     \x20                              installed.\n\
     \x20 --time-budget SECONDS        Give up after this long and report the run as\n\
     \x20                              unfinished, rather than exploring forever.\n\
     \x20                              Defaults to 60s, per (invariant, mutator) pair for\n\
     \x20                              `induct`. An unfinished run counts against a proof,\n\
     \x20                              never for it.\n\
     \x20 --max-steps N                The same, counted in evaluation steps instead\n\
     \x20                              (default 2000000). Steps vary wildly in cost, so\n\
     \x20                              --time-budget is usually the one you want.\n\
     \n\
     SPECIFICATIONS:\n\
     \x20 A specification is a Clarity comment directly above a function, inside a\n\
     \x20 (@clairvoyance ...) block. See `clairvoyance/src/sym/command.clar` for the\n\
     \x20 grammar. In short:\n\
     \x20   (invariant RESULT CONCLUSION)  -- every state returning RESULT must imply CONCLUSION\n\
     \x20   (halt (result ...) (condition ...) (map-write ...) ...)  -- also pin the state written\n\
     \x20 A function with no specification is explored but not judged (reported as NO SPEC).\n\
     ".to_string()
}
