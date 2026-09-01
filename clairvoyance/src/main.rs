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

#![allow(deprecated)]
#![allow(unused_imports)]
#![allow(dead_code)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]

#[macro_use]
extern crate stacks_common;

#[macro_use]
extern crate clarity;
extern crate clarity_types;

extern crate serde;
extern crate serde_json;

extern crate num;

pub mod cli;
pub mod core;
pub mod smt;
pub mod sym;

#[cfg(test)]
pub mod tests;

use std::env;
use std::process;

/// Set the engine's log verbosity from `-v`/`-vv`/`--quiet`, defaulting to
/// quiet so a plain run prints only its result. The engine logs through
/// stacks-common, which reads these environment variables the first time it
/// logs -- which is after this runs -- so setting them here is what takes
/// effect. A verbosity flag is consumed from `argv` if present; an explicit
/// STACKS_LOG_*/BLOCKSTACK_DEBUG in the environment always wins.
fn configure_logging(argv: &mut Vec<String>) {
    let already_set = ["STACKS_LOG_TRACE", "STACKS_LOG_DEBUG", "STACKS_LOG_CRITONLY", "BLOCKSTACK_DEBUG"]
        .iter()
        .any(|k| std::env::var(k).is_ok());

    let mut verbosity: Option<&str> = None;
    argv.retain(|a| match a.as_str() {
        "-q" | "--quiet" => { verbosity = Some("quiet"); false }
        "-v" | "--verbose" => { verbosity = Some("info"); false }
        "-vv" | "--debug" => { verbosity = Some("debug"); false }
        _ => true,
    });

    if already_set {
        return;
    }
    // SAFETY: single-threaded startup, before any logging or other thread has
    // read the environment. The engine reads these variables lazily on its
    // first log call, which happens after this returns.
    match verbosity {
        Some("debug") => unsafe { std::env::set_var("STACKS_LOG_DEBUG", "1") },
        Some("info") => { /* leave the engine default (Info) */ }
        // Default and explicit --quiet: only critical output from the engine.
        _ => unsafe { std::env::set_var("STACKS_LOG_CRITONLY", "1") },
    }
}

fn main() {
    let mut argv : Vec<_> = std::env::args().collect();

    let _prog_name = argv.remove(0);
    configure_logging(&mut argv);
    let (exit_code, message) = cli::run_subcommand(&mut argv);
    if exit_code != 0 {
        eprintln!("{}", &message);
    }
    else {
        println!("{}", &message);
    }
    process::exit(exit_code);
}

