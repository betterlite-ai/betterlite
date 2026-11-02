use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use strum::IntoStaticStr;

use crate::dispatch::progress::Panel;
use crate::dispatch::runner::{self, Invocation};
use crate::dispatch::{Attempt, Places, reviewer_role, sandbox, vendor};
use crate::error::GateError;
use crate::spec::agents::{Agents, Rung, Runner};

#[derive(Clone, Copy, IntoStaticStr)]
#[strum(serialize_all = "kebab-case")]
pub(crate) enum Seat {
    Worker,
    Reviewer,
    ReviewerAgain,
    ParityReviewer,
    ParityReviewerAgain,
}

pub(crate) enum Run {
    Answer {
        text: String,
        session: Option<String>,
    },
    Silent,
    TimedOut,
    Limited,
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
) -> Result<Run, GateError> {
    let roles = &attempt.agents.roles;
    let level = attempt.brief.task.review_level;
    let (schema, role, session) = match seat {
        Seat::Reviewer | Seat::ReviewerAgain => (
            "review.schema.json",
            reviewer_role(attempt.agents, level, Panel::Standard),
            None,
        ),
        Seat::ParityReviewer | Seat::ParityReviewerAgain => {
            ("review.schema.json", &roles.parity_reviewer, None)
        }
        Seat::Worker => ("report.schema.json", &roles.worker, attempt.resume),
    };
    let seat_name = <&str>::from(seat);
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
        instructions: &folder.join(&role.instructions),
        log: &log,
        prompt,
        session,
    };
    run_on_seat(attempt.agents, runner, rung, seat, &invocation)
}

fn run_on_seat(
    agents: &Agents,
    runner: &Runner,
    rung: &Rung,
    seat: Seat,
    invocation: &Invocation<'_>,
) -> Result<Run, GateError> {
    let account = vendor::find_vendor(agents, &runner.vendor)?;
    super::maintenance::check(invocation.git_common_dir, "")?;
    let marker = vendor::marker_time(invocation.git_common_dir, &account.id);
    let Some(seats) = vendor::take_seat(invocation.git_common_dir, agents, account, seat)? else {
        return Ok(Run::Limited);
    };
    super::maintenance::check(invocation.git_common_dir, "")?;
    let started = Instant::now();
    let reply = runner::run(runner, rung, invocation)?;
    if let Some(text) = reply.answer {
        vendor::clear_probe(invocation.git_common_dir, &account.id, marker)?;
        drop(seats);
        return Ok(Run::Answer {
            text,
            session: reply.session,
        });
    }
    let stdout = invocation.output.with_extension("stdout");
    let outcome = match vendor::limit_line(account, &[&stdout, invocation.log])? {
        Some(line) => {
            vendor::mark(invocation.git_common_dir, &account.id, &line)?;
            Ok(Run::Limited)
        }
        None if started.elapsed()
            >= Duration::from_secs(60).saturating_mul(rung.timeout_minutes) =>
        {
            Ok(Run::TimedOut)
        }
        None => Ok(Run::Silent),
    };
    drop(seats);
    outcome
}
