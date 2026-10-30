#![cfg(test)]

use std::io;
use std::sync::{Arc, Mutex};

use tracing::subscriber::with_default;

use super::clamped_override;
use crate::scheduled_job_stagger::PTU_ROLLUP_JOB_ID;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn clamp_with_capture(job_id: &str, requested: u32) -> (u32, String) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .finish();
    let clamped = with_default(subscriber, || clamped_override(job_id, requested));
    let output = String::from_utf8(
        captured
            .0
            .lock()
            .expect("capture mutex should not be poisoned")
            .clone(),
    )
    .expect("captured log output should be UTF-8");
    (clamped, output)
}

#[test]
fn job_without_a_horizon_passes_through() {
    let (clamped, output) = clamp_with_capture("no_such_default_job", 10_000);
    assert_eq!(clamped, 10_000);
    assert!(output.is_empty());
}

#[test]
fn override_under_the_horizon_passes_through() {
    let (clamped, output) = clamp_with_capture(PTU_ROLLUP_JOB_ID, 10);
    assert_eq!(clamped, 10);
    assert!(output.is_empty());
}

#[test]
fn override_at_or_over_the_horizon_is_clamped_with_a_warning() {
    let (clamped, output) = clamp_with_capture(PTU_ROLLUP_JOB_ID, 900);
    assert_eq!(clamped, 899);
    assert!(output.contains("clamped"));
}
