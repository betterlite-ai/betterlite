use std::path::PathBuf;

use strum::IntoStaticStr;

use crate::dispatch::runner::{self, Invocation};
use crate::dispatch::{Attempt, reviewer_role};
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
