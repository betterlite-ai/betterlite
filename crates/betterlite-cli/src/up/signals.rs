use std::sync::Mutex;
use std::sync::PoisonError;

use signal_hook::consts::signal::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

use super::error::UpError;
use super::ports::StopSource;

#[derive(Default)]
pub struct SignalStop {
    signals: Mutex<Option<Signals>>,
}

impl StopSource for SignalStop {
    fn install(&self) -> Result<(), UpError> {
        let signals =
            Signals::new([SIGINT, SIGTERM]).map_err(|source| UpError::Signals { source })?;
        let mut slot = self.signals.lock().unwrap_or_else(PoisonError::into_inner);
        *slot = Some(signals);
        Ok(())
    }

    fn wait(&self) {
        let mut slot = self.signals.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(signals) = slot.as_mut() else {
            return;
        };
        let _unused = signals.wait();
    }
}
