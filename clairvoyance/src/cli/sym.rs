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
    explore_all: bool
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
        .skip_pure(!explore_all)
        .skip_causally_independent(!explore_all);

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
        full_explore_opt.is_some()
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

/// `sym check` -- verify a function (or every function) against the
/// (@clairvoyance ...) specification in its doc comment, and report a
/// developer-facing PASS / VIOLATED / SPEC ERROR verdict with a meaningful
/// exit code. This is the ergonomic front door onto the same engine that
/// `exec-func` drives; the difference is the output and the exit code.
fn cli_check(argv: &[String]) -> (i32, String) {
    let mut remaining_args = argv.to_vec();

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
            full_explore_opt.is_some(),
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
