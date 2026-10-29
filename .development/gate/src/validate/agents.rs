use std::path::Path;

use crate::ids::check_id_problem;
use crate::spec::agents::{Agents, PLACEHOLDERS, Role, Runner, Vendor};
use crate::validate::duplicates;

fn runner_problems(runner: &Runner) -> Vec<String> {
    let mut problems: Vec<String> = check_id_problem(&runner.id)
        .map(|problem| format!("agents.yaml: {problem}"))
        .into_iter()
        .collect();
    for argument in &runner.args {
        let mut rest = argument.as_str();
        while let Some((_, after_open)) = rest.split_once('{') {
            let Some((name, after_close)) = after_open.split_once('}') else {
                problems.push(format!(
                    "agents.yaml: {} has an unclosed placeholder in {argument}",
                    runner.id
                ));
                break;
            };
            if !PLACEHOLDERS.contains(&name) {
                problems.push(format!(
                    "agents.yaml: {} uses unknown placeholder {{{name}}}",
                    runner.id
                ));
            }
            rest = after_close;
        }
    }
    problems
}

fn role_problems(folder: &Path, name: &str, role: &Role, runners: &[&str]) -> Vec<String> {
    let mut problems = Vec::new();
    if !folder.join(&role.instructions).is_file() {
        problems.push(format!(
            "agents.yaml: the {name} instructions .development/{} do not exist",
            role.instructions
        ));
    }
    for rung in &role.ladder {
        if !runners.contains(&rung.runner.as_str()) {
            problems.push(format!(
                "agents.yaml: the {name} ladder names unknown runner {}",
                rung.runner
            ));
        }
        if rung.attempts == 0 || rung.timeout_minutes == 0 || rung.model.trim().is_empty() {
            problems.push(format!(
                "agents.yaml: every {name} rung needs a model, attempts and a timeout above 0"
            ));
        }
    }
    problems
}

fn writable_problems(agents: &Agents) -> Vec<String> {
    let mut problems = Vec::new();
    for entry in &agents.policy.writable {
        let mut rest = entry.as_str();
        while let Some((_, after_open)) = rest.split_once('{') {
            let Some((name, after_close)) = after_open.split_once('}') else {
                problems.push(format!(
                    "agents.yaml: the writable entry {entry} has an unclosed placeholder"
                ));
                break;
            };
            if name != "git_common_dir" && name != "home" {
                problems.push(format!(
                    "agents.yaml: the writable entry {entry} may use only {{git_common_dir}} and {{home}}"
                ));
            }
            rest = after_close;
        }
    }
    problems
}

fn vendor_entry_problems(vendor: &Vendor) -> Vec<String> {
    let mut problems: Vec<String> = check_id_problem(&vendor.id)
        .map(|problem| format!("agents.yaml: {problem}"))
        .into_iter()
        .collect();
    if vendor
        .limit_markers
        .iter()
        .any(|marker| marker.trim().is_empty())
    {
        problems.push(format!(
            "agents.yaml: vendor {} has an empty limit marker; it would match every line",
            vendor.id
        ));
    }
    if vendor.max_parallel == Some(0) {
        problems.push(format!(
            "agents.yaml: vendor {} needs a max_parallel above 0",
            vendor.id
        ));
    }
    problems
}

fn vendor_list_problems(agents: &Agents) -> Vec<String> {
    let vendor_ids: Vec<&str> = agents
        .vendors
        .iter()
        .map(|vendor| vendor.id.as_str())
        .collect();
    let mut problems: Vec<String> = duplicates(&vendor_ids)
        .into_iter()
        .map(|id| format!("agents.yaml defines vendor {id} twice"))
        .collect();
    problems.extend(agents.vendors.iter().flat_map(vendor_entry_problems));
    for runner in &agents.runners {
        if !vendor_ids.contains(&runner.vendor.as_str()) {
            problems.push(format!(
                "agents.yaml: runner {} names unknown vendor {}",
                runner.id, runner.vendor
            ));
        }
    }
    let policy = &agents.policy;
    if policy.max_worker_attempts == 0
        || policy.limit_retry_minutes == 0
        || policy.max_parallel == 0
    {
        problems.push(
            "agents.yaml: max_worker_attempts, limit_retry_minutes and max_parallel must be above 0"
                .to_owned(),
        );
    }
    problems
}

fn vendor_problems(agents: &Agents) -> Vec<String> {
    let vendor = |runner_id: &str| {
        agents
            .runners
            .iter()
            .find(|runner| runner.id == runner_id)
            .map(|runner| runner.vendor.as_str())
    };
    let mut problems = Vec::new();
    let reviewers = [
        ("reviewer", &agents.roles.reviewer),
        ("security reviewer", &agents.roles.security_reviewer),
    ];
    for rung in &agents.roles.worker.ladder {
        let worker_vendor = vendor(&rung.runner);
        for (name, role) in reviewers {
            let covered = role
                .ladder
                .iter()
                .any(|reviewer| vendor(&reviewer.runner) != worker_vendor);
            if !covered {
                problems.push(format!(
                    "agents.yaml: no {name} runs on another vendor than the {} worker rung",
                    rung.model
                ));
            }
        }
    }
    problems
}

pub(crate) fn problems(folder: &Path, agents: &Agents) -> Vec<String> {
    let runner_ids: Vec<&str> = agents
        .runners
        .iter()
        .map(|runner| runner.id.as_str())
        .collect();
    let mut problems: Vec<String> = duplicates(&runner_ids)
        .into_iter()
        .map(|id| format!("agents.yaml defines runner {id} twice"))
        .collect();
    problems.extend(agents.runners.iter().flat_map(runner_problems));
    let roles = [
        ("worker", &agents.roles.worker),
        ("reviewer", &agents.roles.reviewer),
        ("security reviewer", &agents.roles.security_reviewer),
        ("planner", &agents.roles.planner),
    ];
    for (name, role) in roles {
        problems.extend(role_problems(folder, name, role, &runner_ids));
    }
    problems.extend(vendor_list_problems(agents));
    if agents.policy.review_by_other_vendor {
        problems.extend(vendor_problems(agents));
    }
    problems.extend(writable_problems(agents));
    problems
}
