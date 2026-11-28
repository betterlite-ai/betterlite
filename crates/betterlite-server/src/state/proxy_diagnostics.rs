use std::path::PathBuf;
use std::sync::Arc;

use betterlite_config::types::Config;
use betterlite_reporting::bug_report::environment::DOCKERENV_PATH;
use betterlite_reporting::safe_proxy_config::ProxyConfigSnapshot;

use crate::api::proxy_diagnostics::BugReportNotices;

mod tests;

pub struct Slot {
    pub notices: Arc<BugReportNotices>,
}

impl Slot {
    #[must_use]
    pub fn new(config: &Config) -> Self {
        Self {
            notices: Arc::new(BugReportNotices {
                enabled: !config.sections.bug_report_link.disabled,
                // empty until a running config map reaches the server; docs/deviations/B10218.yaml says why
                snapshot: ProxyConfigSnapshot::default(),
                dockerenv: PathBuf::from(DOCKERENV_PATH),
            }),
        }
    }
}
