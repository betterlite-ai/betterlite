use std::io;

use super::refusal::render_refusal;
use super::verdict::{BootVerdict, KeyOrigin, OVERRIDE_SETTING, UnsafeReason};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverrideWarning {
    pub setting: &'static str,
    pub message: String,
}

pub trait BootReporter {
    fn warn(&mut self, warning: OverrideWarning);

    fn announce(&mut self, message: &str) -> Result<(), io::Error>;
}

#[derive(Debug, thiserror::Error)]
pub enum BootError {
    #[error("betterlite refused to start: {reason}")]
    Refused {
        reason: UnsafeReason,
        origin: KeyOrigin,
    },
    #[error("betterlite refused to start: {reason} The fix could not be printed.")]
    Announce {
        reason: UnsafeReason,
        #[source]
        source: io::Error,
    },
}

pub fn enforce_boot_verdict(
    verdict: &BootVerdict,
    reporter: &mut dyn BootReporter,
) -> Result<(), BootError> {
    match verdict {
        BootVerdict::Safe => Ok(()),
        BootVerdict::Allowed { reason } => {
            let state = reason.unsafe_state();
            reporter.warn(OverrideWarning {
                setting: OVERRIDE_SETTING,
                message: format!(
                    "{OVERRIDE_SETTING} is on, so betterlite is starting with {state}. Never run this outside local development."
                ),
            });
            Ok(())
        }
        BootVerdict::Refused(refusal) => {
            let text = render_refusal(refusal);
            reporter
                .announce(&format!("\n{text}\n\n"))
                .map_err(|source| BootError::Announce {
                    reason: refusal.reason,
                    source,
                })?;
            Err(BootError::Refused {
                reason: refusal.reason,
                origin: refusal.origin.clone(),
            })
        }
    }
}
