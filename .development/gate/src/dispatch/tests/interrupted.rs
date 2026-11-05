#![cfg(test)]

use std::fs;

use crate::dispatch::run_ladder;
use crate::spec::agents::OutputMode;

use super::{brief, fixtures, places, progress, worktree};

#[test]
fn a_partial_toy_worker_resumes_its_thread_after_ending_without_a_report() {
    let (mut agents, task) = fixtures();
    agents.roles.worker.ladder.truncate(1);
    agents.roles.worker.ladder[0].attempts = 2;
    for runner in &mut agents.runners {
        "sh".clone_into(&mut runner.program);
        runner.output = OutputMode::File;
        runner.args = vec!["-c".to_owned(),
            r#"cat >/dev/null; echo start >> '{home}/started'; printf '{"type":"thread.started","thread_id":"goose-thread"}\n'"#.to_owned()];
        runner.resume = Some(vec!["-c".to_owned(),
            "cat >/dev/null; echo '{session}' >> '{home}/resumed'; printf 'not a report' > '{output}'".to_owned()]);
    }
    let folder = tempfile::tempdir().unwrap();
    worktree(folder.path());
    let mut saved = progress();
    run_ladder(
        &agents,
        &brief(&task),
        &places(folder.path()),
        &mut saved,
        0,
    )
    .unwrap_err();
    assert_eq!(
        fs::read_to_string(folder.path().join("started")).unwrap(),
        "start\n"
    );
    assert_eq!(
        fs::read_to_string(folder.path().join("resumed")).unwrap(),
        "goose-thread\n".repeat(3)
    );
}
