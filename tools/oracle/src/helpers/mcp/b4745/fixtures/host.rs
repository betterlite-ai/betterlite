use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_mcp::client::tools::progress::ProgressHost;

use super::super::input::HostKind;
use super::types::SimulatedError;

#[derive(Default)]
pub(crate) struct RecordingHost {
    pub(crate) seen: Mutex<Vec<(f64, Option<f64>)>>,
    pub(crate) fail: bool,
}

impl RecordingHost {
    pub(crate) fn seen(&self) -> Vec<(f64, Option<f64>)> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ProgressHost for RecordingHost {
    type Error = SimulatedError;

    fn report(
        &self,
        progress: f64,
        total: Option<f64>,
    ) -> impl Future<Output = Result<(), SimulatedError>> + Send {
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        seen.push((progress, total));
        drop(seen);
        ready(if self.fail {
            Err(SimulatedError("host"))
        } else {
            Ok(())
        })
    }
}

pub(crate) fn host_for(kind: HostKind) -> Option<RecordingHost> {
    match kind {
        HostKind::None => None,
        HostKind::Record => Some(RecordingHost::default()),
        HostKind::Fail => Some(RecordingHost {
            seen: Mutex::new(Vec::new()),
            fail: true,
        }),
    }
}
