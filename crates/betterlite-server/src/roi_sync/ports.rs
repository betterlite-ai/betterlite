use std::sync::Arc;

use betterlite_roi::settings::ROISettings;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::roi_github::client::GitHub;
use crate::roi_github::error::SourceError;
use crate::roi_github::transport::Transport;

pub type GitHubFactory<T> =
    Arc<dyn Fn(&ROISettings, Option<T>) -> Result<GitHub<T>, SourceError> + Send + Sync>;

pub type SyncClock = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

pub type UuidSource = Arc<dyn Fn() -> Uuid + Send + Sync>;
