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
        *self.signals.lock().unwrap_or_else(PoisonError::into_inner) = Some(signals);
        Ok(())
    }

    fn wait(&self) {
        let _unused = self
            .signals
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
            .map(|signals| signals.wait().count());
    }
}
