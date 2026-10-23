use std::path::PathBuf;

use strum::IntoStaticStr;

use crate::dispatch::Attempt;
use crate::dispatch::runner::{self, Invocation};
use crate::error::GateError;
use crate::spec::agents::{Rung, Runner};

#[derive(Clone, Copy, IntoStaticStr)]
#[strum(serialize_all = "kebab-case")]
pub(crate) enum Seat {
    Worker,
    Reviewer,
}

pub(crate) fn invoke(
    attempt: &Attempt<'_>,
    runner: &Runner,
    rung: &Rung,
    seat: Seat,
    prompt: &str,
) -> Result<Option<String>, GateError> {
    let roles = &attempt.agents.roles;
    let (schema, instructions) = match seat {
        Seat::Reviewer => ("review.schema.json", roles.reviewer.instructions.as_str()),
        Seat::Worker => ("report.schema.json", roles.worker.instructions.as_str()),
    };
    let role: &str = seat.into();
    let stem = attempt
        .places
        .journal
        .join(format!("attempt-{}-{role}", attempt.number));
    let output: PathBuf = stem.with_extension("json");
    let log: PathBuf = stem.with_extension("log");
    let places = attempt.places;
    let folder = places.worktree.join(".development");
    let invocation = Invocation {
        worktree: places.worktree,
        git_common_dir: places.git_common_dir,
        home: places.home,
        schema: &folder.join("schema").join(schema),
        output: &output,
        instructions: &folder.join(instructions),
        log: &log,
        prompt,
    };
    runner::run(runner, rung, &invocation)
}
