use std::sync::Arc;

use betterlite_config::types::Config;

use crate::jobs::ptu::PTU_ROLLUP_SCHEDULE;
use crate::jobs::ptu::PtuRollupSchedule;
use crate::jobs::ptu::attribution::{EnvironmentAttribution, PtuAttribution};
use crate::state::shared::Shared;

// The PTU adapter's typed startup resource: the job B8372 registers and the attribution source that decides whether it exists.
pub struct Slot {
    pub schedule: PtuRollupSchedule,
    pub attribution: Arc<dyn PtuAttribution>,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config, _shared: &Shared) -> Self {
        Self {
            schedule: PTU_ROLLUP_SCHEDULE,
            attribution: Arc::new(EnvironmentAttribution {
                configured: config.environment_variables.clone(),
            }),
        }
    }
}
