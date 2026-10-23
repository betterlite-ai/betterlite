mod tests;

use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::answer::parse_answer;
use crate::checks;
use crate::dependencies::added_dependencies;
use crate::development::Development;
use crate::error::GateError;
use crate::facts::resolve_base;
use crate::git::git;
use crate::ids::task_id_problem;
use crate::journal::journal_dir;
use crate::report_check::report_problems;
use crate::spec::agents::Agents;
use crate::spec::report::{Report, ReportStatus};
use crate::yaml::read_yaml;

#[derive(Debug, Deserialize)]
struct HookInput {
    cwd: PathBuf,
    #[serde(default)]
    last_assistant_message: Option<String>,
}

#[derive(Debug, Serialize)]
struct Block<'reason> {
    decision: &'static str,
    reason: &'reason str,
}

pub(crate) fn task_from_branch(branch: &str) -> Option<&str> {
    let rest = branch.strip_prefix("task/")?;
    let (id, tail) = rest.split_at_checked(4)?;
    (task_id_problem(id).is_none() && (tail.is_empty() || tail.starts_with('-'))).then_some(id)
}

fn block(reason: &str) -> Result<Vec<String>, GateError> {
    let decision = Block {
        decision: "block",
        reason,
    };
    let json = serde_json::to_string(&decision).map_err(|source| GateError::Json {
        what: "the hook decision".to_owned(),
        source,
    })?;
    Ok(vec![json])
}

fn block_counted(
    development: &Development,
    task_id: &str,
    lines: &[String],
) -> Result<Vec<String>, GateError> {
    let agents: Agents = read_yaml(&development.root.join(".development/agents.yaml"))?;
    let counter = journal_dir(&development.root, task_id)?.join("hook-blocks");
    let previous = match fs::read_to_string(&counter) {
        Ok(text) => text.trim().parse::<u32>().unwrap_or_default(),
        Err(source) if source.kind() == ErrorKind::NotFound => 0,
        Err(source) => {
            return Err(GateError::Read {
                path: counter,
                source,
            });
        }
    };
    let count = previous.saturating_add(1);
    fs::write(&counter, count.to_string()).map_err(|source| GateError::Write {
        path: counter.clone(),
        source,
    })?;
    if count > agents.policy.max_hook_blocks {
        return Ok(Vec::new());
    }
    let closing = "Fix every problem above, commit, and stop again. When a problem cannot be fixed inside this task, answer with status blocked and the evidence in blocker.";
    block(&format!("{}\n{closing}", lines.join("\n")))
}

pub(crate) fn hook(input: &str) -> Result<Vec<String>, GateError> {
    let payload: HookInput = serde_json::from_str(input).map_err(|source| GateError::Json {
        what: "the hook input".to_owned(),
        source,
    })?;
    let development = Development::open(&payload.cwd)?;
    let branch = git(&development.root, &["branch", "--show-current"])?;
    let Some(task_id) = task_from_branch(branch.trim()) else {
        return block(&format!(
            "Work happens on a branch named task/<ID>-<slug>; this worktree is on '{}'. Switch to it with git switch -c, commit, and stop again.",
            branch.trim()
        ));
    };
    let message = payload.last_assistant_message.unwrap_or_default();
    let report: Report = match parse_answer(&message, "the final answer") {
        Ok(report) => report,
        Err(error) => {
            let line = format!(
                "The final answer must be the report JSON from .development/schema/report.schema.json: {error}"
            );
            return block_counted(&development, task_id, &[line]);
        }
    };
    if report.status == ReportStatus::Blocked && !report.blocker.trim().is_empty() {
        return Ok(Vec::new());
    }
    let verdict = checks::run(&development, &development.task(task_id)?, &[])?;
    let added = added_dependencies(&development.root, &resolve_base(&development.root)?)?;
    let problems = report_problems(&report, task_id, &added);
    if verdict.passed && problems.is_empty() {
        return Ok(Vec::new());
    }
    let lines: Vec<String> = verdict.lines.into_iter().chain(problems).collect();
    block_counted(&development, task_id, &lines)
}
