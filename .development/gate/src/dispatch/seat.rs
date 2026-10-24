use std::path::{Path, PathBuf};

use strum::IntoStaticStr;

use crate::dispatch::runner::{self, Invocation};
use crate::dispatch::{Attempt, Places, reviewer_role, sandbox};
use crate::error::GateError;
use crate::spec::agents::{Rung, Runner};

#[derive(Clone, Copy, IntoStaticStr)]
#[strum(serialize_all = "kebab-case")]
pub(crate) enum Seat {
    Worker,
    Reviewer,
}

fn prepare_sandbox(places: &Places<'_>, stem: &Path) -> Result<(PathBuf, String), GateError> {
    let worktree = places
        .worktree
        .to_str()
        .ok_or(GateError::NotUnicode("the worktree path"))?;
    let settings = stem.with_extension("sandbox.json");
    sandbox::write_settings(&settings, worktree, places.writable)?;
    let roots = serde_json::to_string(places.writable).map_err(|source| GateError::Json {
        what: "the writable folders".to_owned(),
        source,
    })?;
    Ok((settings, roots))
}

pub(crate) fn invoke(
    attempt: &Attempt<'_>,
    runner: &Runner,
    rung: &Rung,
    seat: Seat,
    prompt: &str,
) -> Result<Option<String>, GateError> {
    let (schema, role) = match seat {
        Seat::Reviewer => (
            "review.schema.json",
            reviewer_role(attempt.agents, attempt.brief.task.review_level),
        ),
        Seat::Worker => ("report.schema.json", &attempt.agents.roles.worker),
    };
    let instructions = role.instructions.as_str();
    let seat_name: &str = seat.into();
    let stem = attempt
        .places
        .journal
        .join(format!("attempt-{}-{seat_name}", attempt.number));
    let output: PathBuf = stem.with_extension("json");
    let log: PathBuf = stem.with_extension("log");
    let places = attempt.places;
    let folder = places.worktree.join(".development");
    let (sandbox_settings, writable_roots) = prepare_sandbox(places, &stem)?;
    let invocation = Invocation {
        worktree: places.worktree,
        git_common_dir: places.git_common_dir,
        home: places.home,
        sandbox_settings: &sandbox_settings,
        writable_roots: &writable_roots,
        schema: &folder.join("schema").join(schema),
        output: &output,
        instructions: &folder.join(instructions),
        log: &log,
        prompt,
    };
    runner::run(runner, rung, &invocation)
}
