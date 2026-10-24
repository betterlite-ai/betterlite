mod answer;
mod backlog;
mod builtins;
mod checks;
mod coverage;
mod dependencies;
mod development;
mod deviations;
mod dispatch;
mod error;
mod facts;
mod git;
mod hook;
mod ids;
mod inventory;
mod journal;
mod process;
mod report_check;
mod review_check;
mod schema;
mod show;
mod spec;
mod trace;
mod validate;
mod yaml;

use std::env;
use std::io::{self, Read as _, Write as _};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::development::Development;
use crate::error::GateError;
use crate::spec::litellm::Litellm;
use crate::yaml::read_yaml;

#[derive(Parser)]
#[command(
    name = "gate",
    version,
    about = "Deterministic checks and agent dispatch for betterlite tasks"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(
        about = "Validate every spec file under .development/ and the committed JSON Schemas"
    )]
    Validate,
    #[command(about = "Print the checks a task runs")]
    List { task: String },
    #[command(about = "Run every check of a task; exit 1 when any check fails")]
    Run {
        task: String,
        #[arg(long, help = "Run only this check id (repeatable)")]
        only: Vec<String>,
    },
    #[command(
        about = "Write the JSON Schemas generated from the format types into .development/schema/"
    )]
    Schema,
    #[command(
        about = "Stop hook for Claude Code and Codex: reads the hook JSON on stdin, prints a block decision while the task is not done"
    )]
    Hook,
    #[command(
        about = "Run a task through the worker ladder and the reviewer until it is ready to merge or needs the architect"
    )]
    Dispatch {
        task: String,
        #[arg(
            long = "note",
            help = "Feedback for the first attempt, for example a finding the last run left open; repeat for more"
        )]
        notes: Vec<String>,
    },
    #[command(
        about = "Write .development/litellm-inventory.txt from a LiteLLM clone that has the pinned commit"
    )]
    Inventory { checkout: PathBuf },
    #[command(
        about = "Print how many LiteLLM paths each backlog entry covers, then every path nothing covers"
    )]
    Coverage {
        #[arg(
            long,
            help = "Print every LiteLLM path with the entries that cover it instead"
        )]
        paths: bool,
    },
    #[command(
        about = "Show which task owns a LiteLLM path or a betterlite file, with the LiteLLM lines it ports and its deviations"
    )]
    #[command(
        about = "Show a backlog entry like a ticket: status, links, what it unblocks, components, coverage and its history from git"
    )]
    Show { task: String },
    Trace {
        #[arg(help = "A path from the repository or LiteLLM root, optionally with :line")]
        target: String,
    },
}

fn listing(development: &Development, task_id: &str) -> Result<Vec<String>, GateError> {
    let task = development.task(task_id)?;
    let builtin = development
        .catalog
        .base
        .builtin
        .iter()
        .map(|check| format!("{:16} builtin", <&str>::from(check.id)));
    let commands = checks::commands(development, &task)?
        .into_iter()
        .map(|check| format!("{:16} {}", check.id, check.run));
    Ok(builtin.chain(commands).collect())
}

fn coverage_report(development: &Development) -> Result<(Vec<String>, bool), GateError> {
    let folder = development.root.join(".development");
    let litellm: Litellm = read_yaml(&folder.join("litellm.yaml"))?;
    let measured = coverage::measure(&folder, &litellm, &development.backlog)?;
    let mut lines: Vec<String> = measured
        .entries
        .iter()
        .map(|(id, count)| format!("{id} {count}"))
        .collect();
    lines.extend(coverage::problems(&measured));
    lines.extend(
        measured
            .uncovered
            .iter()
            .map(|path| format!("uncovered {path}")),
    );
    let passed =
        measured.uncovered.is_empty() && measured.dead_globs.is_empty() && measured.stale.is_none();
    Ok((lines, passed))
}

fn path_report(development: &Development) -> Result<(Vec<String>, bool), GateError> {
    let folder = development.root.join(".development");
    let litellm: Litellm = read_yaml(&folder.join("litellm.yaml"))?;
    Ok((
        coverage::path_owners(&folder, &litellm, &development.backlog)?,
        true,
    ))
}

fn dispatch_command(command: Command) -> Result<(Vec<String>, bool), GateError> {
    let directory = env::current_dir().map_err(|source| GateError::Read {
        path: ".".into(),
        source,
    })?;
    if matches!(command, Command::Hook) {
        let mut input = String::new();
        io::stdin()
            .read_to_string(&mut input)
            .map_err(|source| GateError::Read {
                path: "stdin".into(),
                source,
            })?;
        return Ok((hook::hook(&input)?, true));
    }
    run_command(command, &Development::open(&directory)?)
}

fn run_command(
    command: Command,
    development: &Development,
) -> Result<(Vec<String>, bool), GateError> {
    match command {
        Command::Validate => {
            let problems = validate::validate(&development.root)?;
            let passed = problems.is_empty();
            Ok((
                if passed {
                    vec!["valid".to_owned()]
                } else {
                    problems
                },
                passed,
            ))
        }
        Command::List { task } => Ok((listing(development, &task)?, true)),
        Command::Run { task, only } => {
            let verdict = checks::run(development, &development.task(&task)?, &only)?;
            Ok((verdict.lines, verdict.passed))
        }
        Command::Schema => Ok((schema::write(&development.root)?, true)),
        Command::Dispatch { task, notes } => {
            Ok((dispatch::dispatch(development, &task, notes)?, true))
        }
        Command::Inventory { checkout } => {
            Ok((inventory::write(&development.root, &checkout)?, true))
        }
        Command::Coverage { paths: false } => coverage_report(development),
        Command::Coverage { paths: true } => path_report(development),
        Command::Trace { target } => trace::trace(development, &target),
        Command::Show { task } => show::show(development, &task),
        Command::Hook => Ok((Vec::new(), true)),
    }
}

fn main() -> ExitCode {
    let outcome = dispatch_command(Cli::parse().command);
    let mut stdout = io::stdout().lock();
    match outcome {
        Ok((lines, passed)) => {
            let written = lines.iter().try_for_each(|line| writeln!(stdout, "{line}"));
            match (written, passed) {
                (Ok(()), true) => ExitCode::SUCCESS,
                (Ok(()), false) => ExitCode::FAILURE,
                (Err(_), _) => ExitCode::from(2),
            }
        }
        Err(error) => {
            let mut stderr = io::stderr().lock();
            match writeln!(stderr, "gate: {error}") {
                Ok(()) | Err(_) => ExitCode::from(2),
            }
        }
    }
}
