#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::batch_managed_records::Settings;
use betterlite_db::managed_resource_rows::files::ManagedFileTable;
use rstest::rstest;

use super::{
    BatchCostPoller, BatchDatabaseLookup, BatchFileIds, BatchManagedRecords,
    BatchManagedRecordsInput, BatchManagedRecordsResult, BatchUpdate, ManagedFileLookup,
    ManagedFileLookupError, ScheduledJob, ScheduledJobs,
};

#[derive(Debug, thiserror::Error)]
#[error("the scripted lookup failed")]
struct ScriptedFailure;

#[derive(Clone, Default)]
struct RecordingFiles {
    rows: BTreeMap<String, String>,
    failures: BTreeSet<String>,
    lookups: Arc<Mutex<Vec<String>>>,
}

impl RecordingFiles {
    fn recording(rows: &[(&str, &str)]) -> Self {
        Self {
            rows: rows
                .iter()
                .map(|(raw, unified)| ((*raw).to_owned(), (*unified).to_owned()))
                .collect(),
            failures: BTreeSet::new(),
            lookups: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn failing(rows: &[(&str, &str)], failures: &[&str]) -> Self {
        Self {
            rows: rows
                .iter()
                .map(|(raw, unified)| ((*raw).to_owned(), (*unified).to_owned()))
                .collect(),
            failures: failures.iter().map(|raw| (*raw).to_owned()).collect(),
            lookups: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn lookups(&self) -> Vec<String> {
        self.lookups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn managed_file(raw_file_id: &str, unified_file_id: &str) -> ManagedFileTable {
    ManagedFileTable {
        unified_file_id: unified_file_id.to_owned(),
        file_object: None,
        model_mappings: BTreeMap::new(),
        flat_model_file_ids: vec![raw_file_id.to_owned()],
        created_by: None,
        team_id: None,
        updated_by: None,
        storage_backend: None,
        storage_url: None,
    }
}

#[async_trait]
impl ManagedFileLookup for RecordingFiles {
    async fn find_by_flat_model_file_id(
        &self,
        raw_file_id: &str,
    ) -> Result<Option<ManagedFileTable>, ManagedFileLookupError> {
        self.lookups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(raw_file_id.to_owned());
        if self.failures.contains(raw_file_id) {
            return Err(ManagedFileLookupError::Lookup(Box::new(ScriptedFailure)));
        }
        Ok(self
            .rows
            .get(raw_file_id)
            .map(|unified| managed_file(raw_file_id, unified)))
    }
}

struct NoJobs;

impl ScheduledJobs for NoJobs {
    fn scheduled_job(&self, _name: &str) -> Option<ScheduledJob<'_>> {
        None
    }
}

struct ConfirmedPoller;

impl BatchCostPoller for ConfirmedPoller {
    fn support_confirmed(&self) -> bool {
        true
    }
}

struct UnconfirmedPoller;

impl BatchCostPoller for UnconfirmedPoller {
    fn support_confirmed(&self) -> bool {
        false
    }
}

struct ConfirmedJobs;

impl ScheduledJobs for ConfirmedJobs {
    fn scheduled_job(&self, name: &str) -> Option<ScheduledJob<'_>> {
        if name != "check_batch_cost_job" {
            return None;
        }
        Some(ScheduledJob {
            poller: Some(&ConfirmedPoller),
        })
    }
}

struct UnconfirmedJobs;

impl ScheduledJobs for UnconfirmedJobs {
    fn scheduled_job(&self, name: &str) -> Option<ScheduledJob<'_>> {
        if name != "check_batch_cost_job" {
            return None;
        }
        Some(ScheduledJob {
            poller: Some(&UnconfirmedPoller),
        })
    }
}

struct DetachedJob;

impl ScheduledJobs for DetachedJob {
    fn scheduled_job(&self, name: &str) -> Option<ScheduledJob<'_>> {
        if name != "check_batch_cost_job" {
            return None;
        }
        Some(ScheduledJob { poller: None })
    }
}

fn service<Files, Jobs>(
    files: Option<Files>,
    scheduler: Option<Jobs>,
    enabled: bool,
) -> BatchManagedRecords<Files, Jobs> {
    BatchManagedRecords {
        files,
        scheduler,
        settings: Settings {
            proxy_batch_polling_enabled: enabled,
        },
    }
}

fn ids(
    input_file_id: Option<&str>,
    output_file_id: Option<&str>,
    error_file_id: Option<&str>,
) -> BatchFileIds {
    BatchFileIds {
        input_file_id: input_file_id.map(str::to_owned),
        output_file_id: output_file_id.map(str::to_owned),
        error_file_id: error_file_id.map(str::to_owned),
    }
}

#[tokio::test]
async fn existing_input_id_mapping() {
    let files = RecordingFiles::recording(&[("file-raw-in", "litellm_proxy:file:unified-in")]);
    let observed = files.clone();
    let records = service(Some(files), Some(NoJobs), true);
    let input = BatchManagedRecordsInput::ResolveInput {
        files: ids(Some("file-raw-in"), Some("file-raw-out"), None),
    };
    assert_eq!(
        records.run(&input).await,
        BatchManagedRecordsResult::Resolved(ids(
            Some("litellm_proxy:file:unified-in"),
            Some("file-raw-out"),
            None
        ))
    );
    assert_eq!(observed.lookups(), ["file-raw-in"]);
}

#[tokio::test]
async fn existing_output_error_id__5d60() {
    let files = RecordingFiles::recording(&[
        ("file-raw-out", "litellm_proxy:file:unified-out"),
        ("file-raw-err", "litellm_proxy:file:unified-err"),
    ]);
    let observed = files.clone();
    let records = service(Some(files), Some(NoJobs), true);
    let input = BatchManagedRecordsInput::ResolveOutput {
        files: ids(
            Some("file-raw-in"),
            Some("file-raw-out"),
            Some("file-raw-err"),
        ),
    };
    assert_eq!(
        records.run(&input).await,
        BatchManagedRecordsResult::Resolved(ids(
            Some("file-raw-in"),
            Some("litellm_proxy:file:unified-out"),
            Some("litellm_proxy:file:unified-err")
        ))
    );
    assert_eq!(observed.lookups(), ["file-raw-out", "file-raw-err"]);
}

#[tokio::test]
async fn lookup_failure_keeps_raw_id() {
    let files = RecordingFiles::failing(
        &[("file-raw-in", "litellm_proxy:file:unified-in")],
        &["file-raw-in"],
    );
    let observed = files.clone();
    let records = service(Some(files), Some(NoJobs), true);
    let input = BatchManagedRecordsInput::ResolveInput {
        files: ids(Some("file-raw-in"), None, None),
    };
    assert_eq!(
        records.run(&input).await,
        BatchManagedRecordsResult::Resolved(ids(Some("file-raw-in"), None, None))
    );
    assert_eq!(observed.lookups(), ["file-raw-in"]);
}

#[tokio::test]
async fn missing_row_keeps_raw_id() {
    let files = RecordingFiles::recording(&[]);
    let observed = files.clone();
    let records = service(Some(files), Some(NoJobs), true);
    let input = BatchManagedRecordsInput::ResolveOutput {
        files: ids(None, Some("file-raw-unknown"), None),
    };
    assert_eq!(
        records.run(&input).await,
        BatchManagedRecordsResult::Resolved(ids(None, Some("file-raw-unknown"), None))
    );
    assert_eq!(observed.lookups(), ["file-raw-unknown"]);
}

#[tokio::test]
async fn empty_and_unified_ids_are_not_looked_up() {
    let unified = URL_SAFE_NO_PAD.encode("litellm_proxy:file:unified-in");
    let files = RecordingFiles::recording(&[]);
    let observed = files.clone();
    let records = service(Some(files), Some(NoJobs), true);
    let input = BatchManagedRecordsInput::Ensure {
        files: ids(Some(""), Some(&unified), None),
    };
    assert_eq!(
        records.run(&input).await,
        BatchManagedRecordsResult::Ensured(ids(Some(""), Some(&unified), None))
    );
    assert!(observed.lookups().is_empty());
}

#[tokio::test]
async fn ensure_no_hook_stops_afte_1794() {
    let files = RecordingFiles::recording(&[
        ("file-raw-in", "litellm_proxy:file:unified-in"),
        ("file-raw-out", "litellm_proxy:file:unified-out"),
        ("file-raw-err", "litellm_proxy:file:unified-err"),
    ]);
    let observed = files.clone();
    let records = service(Some(files), Some(NoJobs), true);
    let input = BatchManagedRecordsInput::Ensure {
        files: ids(
            Some("file-raw-in"),
            Some("file-raw-out"),
            Some("file-raw-err"),
        ),
    };
    assert_eq!(
        records.run(&input).await,
        BatchManagedRecordsResult::Ensured(ids(
            Some("litellm_proxy:file:unified-in"),
            Some("litellm_proxy:file:unified-out"),
            Some("litellm_proxy:file:unified-err")
        ))
    );
    assert_eq!(
        observed.lookups(),
        ["file-raw-in", "file-raw-out", "file-raw-err"]
    );
}

// the absent hook returns (None, None) before the source reads a managed-object row
#[tokio::test]
async fn database_get_no_hook_no_query() {
    let files = RecordingFiles::recording(&[("file-raw-out", "litellm_proxy:file:unified-out")]);
    let observed = files.clone();
    let records = service(Some(files), Some(NoJobs), true);
    let input = BatchManagedRecordsInput::DatabaseGet {
        batch_id: Some("batch-1".to_owned()),
        unified_batch_id: Some("litellm_proxy:batch:1".to_owned()),
    };
    assert_eq!(
        records.run(&input).await,
        BatchManagedRecordsResult::DatabaseGet(BatchDatabaseLookup {
            db_batch_object: None,
            response: None,
        })
    );
    assert!(observed.lookups().is_empty());
}

// the same guard returns before the response is normalized or a row is written
#[tokio::test]
async fn database_update_no_hook_n_99fb() {
    let files = RecordingFiles::recording(&[
        ("file-raw-in", "litellm_proxy:file:unified-in"),
        ("file-raw-out", "litellm_proxy:file:unified-out"),
    ]);
    let observed = files.clone();
    let records = service(Some(files), Some(NoJobs), true);
    let input = BatchManagedRecordsInput::DatabaseUpdate {
        batch_id: Some("batch-1".to_owned()),
        unified_batch_id: Some("litellm_proxy:batch:1".to_owned()),
        files: ids(Some("file-raw-in"), Some("file-raw-out"), None),
    };
    assert_eq!(
        records.run(&input).await,
        BatchManagedRecordsResult::DatabaseUpdate(BatchUpdate {
            files: ids(Some("file-raw-in"), Some("file-raw-out"), None),
            written: false,
        })
    );
    assert!(observed.lookups().is_empty());
}

#[tokio::test]
async fn poller_job_absent() {
    let records = service(None::<RecordingFiles>, Some(NoJobs), true);
    let result = records.run(&BatchManagedRecordsInput::PollerActive).await;
    assert_eq!(result, BatchManagedRecordsResult::PollerActive(false));
}

#[tokio::test]
async fn poller_with_confirmed_job_is_active() {
    let records = service(None::<RecordingFiles>, Some(ConfirmedJobs), true);
    let result = records.run(&BatchManagedRecordsInput::PollerActive).await;
    assert_eq!(result, BatchManagedRecordsResult::PollerActive(true));
}

#[tokio::test]
async fn poller_disabled_with_confirmed_job_is_inactive() {
    let records = service(None::<RecordingFiles>, Some(ConfirmedJobs), false);
    let result = records.run(&BatchManagedRecordsInput::PollerActive).await;
    assert_eq!(result, BatchManagedRecordsResult::PollerActive(false));
}

#[tokio::test]
async fn poller_without_scheduler_is_inactive() {
    let records = service(None::<RecordingFiles>, None::<ConfirmedJobs>, true);
    let result = records.run(&BatchManagedRecordsInput::PollerActive).await;
    assert_eq!(result, BatchManagedRecordsResult::PollerActive(false));
}

#[tokio::test]
async fn poller_with_unconfirmed_job_is_inactive() {
    let records = service(None::<RecordingFiles>, Some(UnconfirmedJobs), true);
    let result = records.run(&BatchManagedRecordsInput::PollerActive).await;
    assert_eq!(result, BatchManagedRecordsResult::PollerActive(false));
}

#[tokio::test]
async fn poller_with_unbound_job_function_is_inactive() {
    let records = service(None::<RecordingFiles>, Some(DetachedJob), true);
    let result = records.run(&BatchManagedRecordsInput::PollerActive).await;
    assert_eq!(result, BatchManagedRecordsResult::PollerActive(false));
}

#[rstest]
#[case::unset_defaults_on(&[], Some(true))]
#[case::explicit_true(&[("PROXY_BATCH_POLLING_ENABLED", "true")], Some(true))]
#[case::explicit_false(&[("PROXY_BATCH_POLLING_ENABLED", "false")], Some(false))]
#[case::empty_is_unset(&[("PROXY_BATCH_POLLING_ENABLED", "")], Some(true))]
#[case::invalid_value_names_the_variable(
    &[("PROXY_BATCH_POLLING_ENABLED", "perhaps")],
    None
)]
#[tokio::test]
async fn loads_settings(#[case] environment: &[(&str, &str)], #[case] expected: Option<bool>) {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let loaded = ConfigLoader { lookup }.load("{}\n");
    let Some(enabled) = expected else {
        let error = loaded.unwrap_err();
        assert!(
            error.to_string().contains("PROXY_BATCH_POLLING_ENABLED"),
            "{error}"
        );
        return;
    };
    let settings = loaded.unwrap().sections.batch_managed_records;
    assert_eq!(settings.proxy_batch_polling_enabled, enabled);
    let records = BatchManagedRecords {
        files: None::<RecordingFiles>,
        scheduler: Some(ConfirmedJobs),
        settings,
    };
    assert_eq!(
        records.run(&BatchManagedRecordsInput::PollerActive).await,
        BatchManagedRecordsResult::PollerActive(enabled)
    );
}
