use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::sections::batch_managed_records::Settings;
use betterlite_core::api::batch_managed_records::{
    BatchFileIds, BatchManagedRecords, BatchManagedRecordsInput, BatchManagedRecordsResult,
    BatchStoredObject, ManagedFileLookup, ScheduledJob, ScheduledJobs,
};
use betterlite_db::managed_resource_rows::files::ManagedFileTable;
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    plan: BatchManagedRecordsInput,
    database: Database,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Database {
    present: bool,
    #[serde(default)]
    found: BTreeMap<String, String>,
    #[serde(default)]
    failures: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("the scripted lookup failed for {raw_file_id}")]
struct ScriptedFailure {
    raw_file_id: String,
}

struct ScriptedFiles {
    found: BTreeMap<String, String>,
    failures: BTreeSet<String>,
    lookups: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl ManagedFileLookup for ScriptedFiles {
    type Error = ScriptedFailure;

    async fn find_by_flat_model_file_id(
        &self,
        raw_file_id: &str,
    ) -> Result<Option<ManagedFileTable>, ScriptedFailure> {
        self.lookups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(raw_file_id.to_owned());
        if self.failures.contains(raw_file_id) {
            return Err(ScriptedFailure {
                raw_file_id: raw_file_id.to_owned(),
            });
        }
        Ok(self
            .found
            .get(raw_file_id)
            .map(|unified_file_id| ManagedFileTable {
                unified_file_id: unified_file_id.clone(),
                file_object: None,
                model_mappings: BTreeMap::new(),
                flat_model_file_ids: vec![raw_file_id.to_owned()],
                created_by: None,
                team_id: None,
                updated_by: None,
                storage_backend: None,
                storage_url: None,
            }))
    }
}

struct NoScheduledJobs;

impl ScheduledJobs for NoScheduledJobs {
    fn scheduled_job(&self, _name: &str) -> Option<ScheduledJob<'_>> {
        None
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let lookups = Arc::new(Mutex::new(Vec::new()));
        let files = input.database.present.then(|| ScriptedFiles {
            found: input.database.found.clone(),
            failures: input.database.failures.iter().cloned().collect(),
            lookups: Arc::clone(&lookups),
        });
        // LiteLLM reads PROXY_BATCH_POLLING_ENABLED once at import, so a helper process runs the default
        let records = BatchManagedRecords {
            files,
            scheduler: Some(NoScheduledJobs),
            settings: Settings::default(),
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        let outcome = runtime.block_on(records.run(&input.plan));
        let seen = lookups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Ok(json!({
            "result": result_value(&outcome),
            "events": event_values(&seen),
        }))
    }
}

fn files_value(files: &BatchFileIds) -> Value {
    json!({
        "input_file_id": &files.input_file_id,
        "output_file_id": &files.output_file_id,
        "error_file_id": &files.error_file_id,
    })
}

fn stored_object_value(stored: Option<&BatchStoredObject>) -> Value {
    stored.map_or(Value::Null, |object| {
        json!({
            "unified_object_id": &object.unified_object_id,
            "status": &object.status,
        })
    })
}

fn event_values(lookups: &[String]) -> Value {
    json!(
        lookups
            .iter()
            .map(|raw_id| json!({"event": "file_lookup", "raw_id": raw_id}))
            .collect::<Vec<Value>>()
    )
}

fn result_value(result: &BatchManagedRecordsResult) -> Value {
    match result {
        BatchManagedRecordsResult::Resolved(files) | BatchManagedRecordsResult::Ensured(files) => {
            files_value(files)
        }
        BatchManagedRecordsResult::DatabaseGet(lookup) => json!({
            "db_batch_object": stored_object_value(lookup.db_batch_object.as_ref()),
            "response": lookup.response.as_ref().map_or(Value::Null, files_value),
        }),
        BatchManagedRecordsResult::DatabaseUpdate(update) => json!({
            "files": files_value(&update.files),
            "written": update.written,
        }),
        BatchManagedRecordsResult::PollerActive(active) => json!({"active": active}),
    }
}
