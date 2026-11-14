mod affected;
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
mod exceptions;
mod facts;
mod gate_lock;
mod git;
mod hook;
mod ids;
mod inventory;
mod journal;
mod process;
mod report_check;
mod review_check;
mod schema;
mod shellcheck;
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

use crate::checks::Selection;
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
        about = "Print the nextest package flags for the packages a change against GATE_BASE can affect: --workspace, -p flags, or nothing; GATE_ALL_TESTS set takes every package"
    )]
    Affected {
        #[arg(
            long,
            help = "Print only the packages the change edits, without the ones that depend on them"
        )]
        edited: bool,
        #[command(flatten)]
        shard: affected::Shard,
    },
    #[command(
        about = "Validate every spec file under .development/ and the committed JSON Schemas"
    )]
    Validate,
    #[command(
        about = "Print every exception to check later as CSV: tasks merged with new expects or a raised line budget, and the advisories and duplicate crates deny.toml allows"
    )]
    Exceptions,
    #[command(about = "Print the checks a task runs")]
    List {
        task: String,
        #[arg(long, help = "Print only the checks a dispatch runs locally")]
        local: bool,
    },
    #[command(
        about = "Check task prerequisites without running builds, tests, coverage or mutants"
    )]
    Preflight { task: String },
    #[command(about = "Check account backoff before preparing a checkout; starts no agents")]
    Availability,
    #[command(about = "Run every check of a task; exit 1 when any check fails")]
    Run {
        task: String,
        #[arg(
            long,
            value_delimiter = ',',
            help = "Run only these check ids (repeatable or comma-separated)"
        )]
        only: Vec<String>,
        #[arg(
            long,
            value_delimiter = ',',
            help = "Leave these check ids out (repeatable or comma-separated)"
        )]
        skip: Vec<String>,
        #[arg(
            long,
            value_delimiter = ',',
            help = "Start these check ids at once, one after another, while the other checks run (repeatable or comma-separated)"
        )]
        beside: Vec<String>,
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
        #[arg(
            long,
            default_value_t = 0,
            help = "Worker starts beyond the ladder, for a task whose ladder ran out; they run on the last rung"
        )]
        grant: u32,
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
        about = "Show a backlog entry like a ticket: status, links, what it unblocks, components, coverage and its history from git"
    )]
    Show { task: String },
    #[command(
        about = "Show which task owns a LiteLLM path or a betterlite file, with the LiteLLM lines it ports and its deviations"
    )]
    Trace {
        #[arg(help = "A path from the repository or LiteLLM root, optionally with :line")]
        target: String,
    },
    #[command(
        about = "Run shellcheck on every run command of checks.yaml, with .shellcheckrc, or of task files, with the default checks"
    )]
    Shellcheck {
        #[arg(
            required = true,
            help = ".development/checks.yaml or a file in .development/tasks/"
        )]
        paths: Vec<PathBuf>,
    },
}

fn listing(
    development: &Development,
    task_id: &str,
    local: bool,
) -> Result<Vec<String>, GateError> {
    let task = development.task(task_id)?;
    let builtin = development
        .catalog
        .base
        .builtin
        .iter()
        .map(|check| format!("{:16} builtin", <&str>::from(check.id)));
    let commands = checks::commands(development, &task)?
        .into_iter()
        .filter(|check| !local || checks::selection::lint(&check.id))
        .map(|check| {
            format!(
                "{:16} {}",
                check.id,
                check.run.replace('\n', "\n                 ")
            )
        });
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
        path: PathBuf::from("."),
        source,
    })?;
    if matches!(command, Command::Hook) {
        let mut input = String::new();
        io::stdin()
            .read_to_string(&mut input)
            .map_err(|source| GateError::Read {
                path: PathBuf::from("stdin"),
                source,
            })?;
        return Ok((hook::hook(&input)?, true));
    }
    run_command(command, &Development::open(&directory)?)
}

fn run_selection(
    development: &Development,
    id: &str,
    selection: &Selection,
) -> Result<(Vec<String>, bool), GateError> {
    let task = development.task(id)?;
    let verdict = checks::run(development, &task, selection)?;
    Ok((verdict.lines, verdict.passed))
}

fn preflight(development: &Development, id: &str) -> Result<(Vec<String>, bool), GateError> {
    let task = development.task(id)?;
    let selection =
        checks::selection::builtins_and(development, &task, checks::selection::prerequisite)?;
    let verdict = checks::run(development, &task, &selection)?;
    Ok((verdict.lines, verdict.passed))
}

fn run_command(
    command: Command,
    development: &Development,
) -> Result<(Vec<String>, bool), GateError> {
    match command {
        Command::Affected { edited, shard } => {
            Ok((affected::affected(&development.root, edited, shard)?, true))
        }
        Command::Availability => {
            dispatch::availability(development)?;
            Ok((vec!["a worker vendor is available".to_owned()], true))
        }
        Command::Validate => validate_command(development),
        Command::Exceptions => Ok((exceptions::exceptions(development)?, true)),
        Command::Shellcheck { paths } => shellcheck::shellcheck(&development.root, &paths),
        Command::List { task, local } => Ok((listing(development, &task, local)?, true)),
        Command::Preflight { task } => preflight(development, &task),
        Command::Run {
            task,
            only,
            skip,
            beside,
        } => run_selection(development, &task, &Selection { only, skip, beside }),
        Command::Schema => Ok((schema::write(&development.root)?, true)),
        Command::Dispatch { task, notes, grant } => {
            Ok((dispatch::dispatch(development, &task, notes, grant)?, true))
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

fn validate_command(development: &Development) -> Result<(Vec<String>, bool), GateError> {
    let problems = validate::validate(&development.root)?;
    let passed = problems.is_empty();
    let lines = if passed {
        vec!["valid".to_owned()]
    } else {
        problems
    };
    Ok((lines, passed))
}
