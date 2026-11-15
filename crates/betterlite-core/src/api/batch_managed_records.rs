use async_trait::async_trait;
use betterlite_config::sections::batch_managed_records::Settings as ConfiguredSettings;
use betterlite_db::managed_resource_rows::files::ManagedFileTable;
use serde::Deserialize;
use thiserror::Error;

use crate::api::managed_resource_ids::is_base64_encoded_unified_id;

mod tests;

// SpecialEnums.LITELM_MANAGED_FILE_ID_PREFIX: a unified file id is base64 text that decodes to this bare prefix
pub const MANAGED_FILE_ID_PREFIX: &str = "litellm_proxy";

// the job name the enterprise poller registers; this build never registers it
pub const CHECK_BATCH_COST_JOB: &str = "check_batch_cost_job";

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct BatchFileIds {
    #[serde(default)]
    pub input_file_id: Option<String>,
    #[serde(default)]
    pub output_file_id: Option<String>,
    #[serde(default)]
    pub error_file_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum BatchManagedRecordsInput {
    ResolveInput {
        files: BatchFileIds,
    },
    ResolveOutput {
        files: BatchFileIds,
    },
    Ensure {
        files: BatchFileIds,
    },
    DatabaseGet {
        #[serde(default)]
        batch_id: Option<String>,
        #[serde(default)]
        unified_batch_id: Option<String>,
    },
    DatabaseUpdate {
        #[serde(default)]
        batch_id: Option<String>,
        #[serde(default)]
        unified_batch_id: Option<String>,
        files: BatchFileIds,
    },
    PollerActive,
}

// the managed-object row get_batch_from_database would return; only the hook that is absent in this build reads it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchStoredObject {
    pub unified_object_id: String,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BatchDatabaseLookup {
    pub db_batch_object: Option<BatchStoredObject>,
    pub response: Option<BatchFileIds>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchUpdate {
    pub files: BatchFileIds,
    pub written: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchManagedRecordsResult {
    Resolved(BatchFileIds),
    Ensured(BatchFileIds),
    DatabaseGet(BatchDatabaseLookup),
    DatabaseUpdate(BatchUpdate),
    PollerActive(bool),
}

#[derive(Debug, Error)]
pub enum ManagedFileLookupError {
    #[error("the managed-file lookup failed")]
    Lookup(#[source] Box<dyn std::error::Error + Send + Sync>),
}

#[async_trait]
pub trait ManagedFileLookup: Send + Sync {
    async fn find_by_flat_model_file_id(
        &self,
        raw_file_id: &str,
    ) -> Result<Option<ManagedFileTable>, ManagedFileLookupError>;
}

pub struct ScheduledJob<'job> {
    pub poller: Option<&'job dyn BatchCostPoller>,
}

pub trait ScheduledJobs: Send + Sync {
    fn scheduled_job(&self, name: &str) -> Option<ScheduledJob<'_>>;
}

pub trait BatchCostPoller: Send + Sync {
    fn support_confirmed(&self) -> bool;
}

pub struct BatchManagedRecords<Files, Jobs> {
    pub files: Option<Files>,
    pub scheduler: Option<Jobs>,
    pub settings: ConfiguredSettings,
}

impl<Files, Jobs> BatchManagedRecords<Files, Jobs>
where
    Files: ManagedFileLookup,
    Jobs: ScheduledJobs,
{
    pub async fn run(&self, input: &BatchManagedRecordsInput) -> BatchManagedRecordsResult {
        match input {
            BatchManagedRecordsInput::ResolveInput { files } => {
                BatchManagedRecordsResult::Resolved(self.resolve_input(files).await)
            }
            BatchManagedRecordsInput::ResolveOutput { files } => {
                BatchManagedRecordsResult::Resolved(self.resolve_output(files).await)
            }
            BatchManagedRecordsInput::Ensure { files } => {
                BatchManagedRecordsResult::Ensured(self.ensure(files).await)
            }
            // no managed-files hook exists in this build, so the source's first guard returns before any read
            BatchManagedRecordsInput::DatabaseGet { .. } => {
                BatchManagedRecordsResult::DatabaseGet(BatchDatabaseLookup::default())
            }
            // the same absent hook returns before the source normalizes the response or writes the row
            BatchManagedRecordsInput::DatabaseUpdate { files, .. } => {
                BatchManagedRecordsResult::DatabaseUpdate(BatchUpdate {
                    files: files.clone(),
                    written: false,
                })
            }
            BatchManagedRecordsInput::PollerActive => {
                BatchManagedRecordsResult::PollerActive(self.batch_cost_poller_is_active())
            }
        }
    }

    async fn resolve_input(&self, files: &BatchFileIds) -> BatchFileIds {
        BatchFileIds {
            input_file_id: self.resolved_id(files.input_file_id.as_deref()).await,
            output_file_id: files.output_file_id.clone(),
            error_file_id: files.error_file_id.clone(),
        }
    }

    async fn resolve_output(&self, files: &BatchFileIds) -> BatchFileIds {
        BatchFileIds {
            input_file_id: files.input_file_id.clone(),
            output_file_id: self.resolved_id(files.output_file_id.as_deref()).await,
            error_file_id: self.resolved_id(files.error_file_id.as_deref()).await,
        }
    }

    async fn ensure(&self, files: &BatchFileIds) -> BatchFileIds {
        let resolved = self.resolve_input(files).await;
        // the managed-files hook is absent in this build, so the source returns right after its two lookups
        self.resolve_output(&resolved).await
    }

    // the source replaces a raw id with the row's unified id, keeps it when the row is missing, and swallows a lookup failure
    async fn resolved_id(&self, raw_file_id: Option<&str>) -> Option<String> {
        let Some(files) = self.files.as_ref() else {
            return raw_file_id.map(str::to_owned);
        };
        let Some(raw_id) = raw_file_id else {
            return None;
        };
        if raw_id.is_empty()
            || is_base64_encoded_unified_id(raw_id, MANAGED_FILE_ID_PREFIX).is_some()
        {
            return Some(raw_id.to_owned());
        }
        match files.find_by_flat_model_file_id(raw_id).await {
            Ok(Some(managed_file)) => Some(managed_file.unified_file_id),
            Ok(None) | Err(_error) => Some(raw_id.to_owned()),
        }
    }

    // false unless polling is on, the scheduler holds the job, and its poller confirmed batch_processed support
    #[must_use]
    pub fn batch_cost_poller_is_active(&self) -> bool {
        if !self.settings.proxy_batch_polling_enabled {
            return false;
        }
        let Some(jobs) = self.scheduler.as_ref() else {
            return false;
        };
        let Some(job) = jobs.scheduled_job(CHECK_BATCH_COST_JOB) else {
            return false;
        };
        job.poller.is_some_and(|poller| poller.support_confirmed())
    }
}
