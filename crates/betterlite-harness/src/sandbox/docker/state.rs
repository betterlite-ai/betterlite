use std::sync::Arc;

use rustix::process::Signal;

use super::process::OwnedChild;

#[derive(Default)]
pub(super) struct State {
    pub(super) container_id: Option<String>,
    pub(super) closed: bool,
    pub(super) processes: Vec<Arc<OwnedChild>>,
}

impl Drop for State {
    fn drop(&mut self) {
        for process in &self.processes {
            drop(process.signal(Signal::KILL));
        }
    }
}
