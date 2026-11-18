use std::time::Duration;

use betterlite_core::api::file_storage_service::projection::{FileMetadata, project_stored_file};
use betterlite_logging::azure_storage_logging::filename::IdSource;
use betterlite_logging::datadog_logging::clock::Clock;
use betterlite_openai::files::records::FilesPurpose;
use serde::Deserialize;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    content: String,
    filename: String,
    purpose: FilesPurpose,
    target_storage: String,
    storage_url: String,
    now: f64,
    uuid: String,
}

struct FixedClock {
    now: f64,
}

impl Clock for FixedClock {
    fn now_unix(&self) -> Duration {
        Duration::try_from_secs_f64(self.now).unwrap_or_default()
    }
}

struct FixedIds<'ids> {
    uuid: &'ids str,
}

impl IdSource for FixedIds<'_> {
    fn next_id(&self) -> String {
        self.uuid.to_owned()
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let stored = project_stored_file(
            FileMetadata {
                content: input.content.as_bytes(),
                filename: &input.filename,
                purpose: input.purpose,
                target_storage: &input.target_storage,
                storage_url: &input.storage_url,
            },
            &FixedClock { now: input.now },
            &FixedIds { uuid: &input.uuid },
        );
        Ok(serde_json::to_value(stored.file)?)
    }
}
