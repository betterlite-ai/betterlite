use betterlite_auth::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use betterlite_models::decisions::Json;
use indexmap::IndexSet;

use super::access::{self, AccessFailure};
use super::error::BatchInputAccountingError;
use super::input::BatchInputAccountingInput;
use super::logging::log_failure;
use super::ports::{
    BatchAccountingCalls, BatchFileFetch, BatchFileRequest, BatchFileSource, BatchModelAccess,
};
use super::result::BatchInputAccountingResult;
use super::rows::{RowInput, countable_entry, entry_model, row_estimate};
use super::totals::{Totals, min_configured_otpm_limit};
use super::{DecodedBatchInputLine, decode_batch_input_accounting};
use crate::api::api_boundaries::ApiLimits;
use crate::api::batch_managed_records::MANAGED_FILE_ID_PREFIX;
use crate::api::batch_output_usage::tokens::{BatchTokenCounter, count_batch_entry_tokens};
use crate::api::managed_resource_ids::{is_base64_encoded_unified_id, target_model_names};
use crate::api::request_destination::RequestData;

// The failure of one call, naming each injected port's own error
type SliceError<Calls, Files, Access> = BatchInputAccountingError<
    <Calls as BatchAccountingCalls>::Error,
    <Files as BatchFileSource>::Error,
    <Access as BatchModelAccess>::Error,
>;

// The file content and the models a managed file was uploaded for
struct LoadedBatch {
    bytes: Vec<u8>,
    target_models: Option<Vec<String>>,
}

// _PROXY_BatchRateLimiter.count_input_file_usage over an already fetched file
pub struct BatchInputAccounting<'slice, Calls, Files, Access> {
    calls: &'slice Calls,
    files: &'slice Files,
    access: &'slice Access,
    counter: &'slice dyn BatchTokenCounter,
    limits: ApiLimits,
}

impl<'slice, Calls, Files, Access> BatchInputAccounting<'slice, Calls, Files, Access>
where
    Calls: BatchAccountingCalls,
    Files: BatchFileSource,
    Access: BatchModelAccess,
{
    #[must_use]
    pub const fn new(
        calls: &'slice Calls,
        files: &'slice Files,
        access: &'slice Access,
        counter: &'slice dyn BatchTokenCounter,
        limits: ApiLimits,
    ) -> Self {
        Self {
            calls,
            files,
            access,
            counter,
            limits,
        }
    }

    pub async fn run(
        &self,
        input: &BatchInputAccountingInput,
    ) -> Result<BatchInputAccountingResult, SliceError<Calls, Files, Access>> {
        let outcome = self.account(input).await;
        if let Err(error) = &outcome {
            log_failure(&input.file_id, error);
        }
        outcome
    }

    async fn account(
        &self,
        input: &BatchInputAccountingInput,
    ) -> Result<BatchInputAccountingResult, SliceError<Calls, Files, Access>> {
        let caller = match &input.caller {
            Some(principal) => Some(UserAPIKeyAuth::construct(AuthInput::Dict(
                principal.0.clone(),
            ))?),
            None => None,
        };
        let min = min_configured_otpm_limit(input, caller.as_ref());
        let loaded = self.load_content(input, caller.as_ref()).await?;
        let lines = decode_batch_input_accounting(&loaded.bytes, &self.limits)?;
        let mut totals = Totals::default();
        let mut models = IndexSet::new();
        for line in &lines {
            self.account_line(line, min, &mut totals, &mut models)
                .await?;
        }
        if let Some(caller) = &caller {
            self.enforce_models(caller, &models, loaded.target_models.as_deref())
                .await?;
        }
        Ok(BatchInputAccountingResult {
            total_tokens: totals.total_tokens,
            request_count: totals.request_count,
            output_tokens: totals.output_tokens,
            per_model_usage: totals.per_model,
        })
    }

    // Managed files go through the caller-aware hook; every other file id is read from its provider
    async fn load_content(
        &self,
        input: &BatchInputAccountingInput,
        caller: Option<&UserAPIKeyAuth>,
    ) -> Result<LoadedBatch, SliceError<Calls, Files, Access>> {
        let managed = is_base64_encoded_unified_id(&input.file_id, MANAGED_FILE_ID_PREFIX);
        let (fetched, target_models) = match (managed, caller) {
            (Some(decoded), Some(caller)) => {
                let names = target_model_names(&decoded);
                let request = BatchFileRequest::Managed {
                    file_id: &input.file_id,
                    caller,
                };
                let fetched = self
                    .files
                    .fetch(request)
                    .await
                    .map_err(BatchInputAccountingError::Files)?;
                (fetched, Some(names))
            }
            _ => {
                let data = input.data.clone().unwrap_or_default();
                let params = self
                    .calls
                    .resolve_fetch_params(&input.file_id, input.custom_llm_provider.slug(), &data)
                    .map_err(BatchInputAccountingError::Calls)?;
                let request = BatchFileRequest::Provider {
                    file_id: &params.file_id,
                    params: &params.params,
                    caller,
                };
                let fetched = self
                    .files
                    .fetch(request)
                    .await
                    .map_err(BatchInputAccountingError::Files)?;
                (fetched, None)
            }
        };
        let bytes = match fetched {
            BatchFileFetch::Content(bytes) => bytes,
            BatchFileFetch::NotBytes { kind } => {
                return Err(BatchInputAccountingError::ContentNotBytes {
                    file_id: input.file_id.clone(),
                    kind,
                });
            }
            BatchFileFetch::ManagedUnavailable(reason) => {
                return Err(BatchInputAccountingError::Managed(reason));
            }
            BatchFileFetch::Http(http) => return Err(BatchInputAccountingError::Http(http)),
        };
        Ok(LoadedBatch {
            bytes,
            target_models: target_models.filter(|names| !names.is_empty()),
        })
    }

    // Every model named in the file is checked once the pass finished, so a bad row cannot hide one
    async fn enforce_models(
        &self,
        caller: &UserAPIKeyAuth,
        models: &IndexSet<String>,
        target_models: Option<&[String]>,
    ) -> Result<(), SliceError<Calls, Files, Access>> {
        access::enforce(self.access, caller, models, target_models)
            .await
            .map_err(|failure| match failure {
                AccessFailure::Fields(source) => BatchInputAccountingError::Fields(source),
                AccessFailure::Source(source) => BatchInputAccountingError::Access(source),
                AccessFailure::Http(http) => BatchInputAccountingError::Http(http),
            })
    }

    async fn account_line(
        &self,
        line: &DecodedBatchInputLine<'_>,
        min: Option<i64>,
        totals: &mut Totals,
        models: &mut IndexSet<String>,
    ) -> Result<(), SliceError<Calls, Files, Access>> {
        totals.request_count = totals.request_count.saturating_add(1);
        match line {
            DecodedBatchInputLine::Entry { entry, raw } => {
                self.account_entry(entry, raw, min, totals, models).await
            }
            DecodedBatchInputLine::Fallback { raw } => {
                let output = self.calls.output_floor(min);
                totals.add(None, row_estimate(raw), output);
                Ok(())
            }
        }
    }

    async fn account_entry(
        &self,
        entry: &RequestData,
        raw: &[u8],
        min: Option<i64>,
        totals: &mut Totals,
        models: &mut IndexSet<String>,
    ) -> Result<(), SliceError<Calls, Files, Access>> {
        let model = entry_model(entry);
        if let Some(model) = model {
            models.insert(model.to_owned());
        }
        let output = self
            .calls
            .estimate_output_tokens(entry, min)
            .map_err(BatchInputAccountingError::Calls)?;
        let total = match countable_entry(entry) {
            RowInput::Zero => 0,
            RowInput::Unmeasurable => row_estimate(raw),
            RowInput::Measured(measured) => self.count_measured(&measured, raw).await?,
        };
        totals.add(model, total, output);
        Ok(())
    }

    // A row the counter cannot measure keeps its size-based estimate, as LiteLLM's per-row except does
    async fn count_measured(
        &self,
        entry: &RequestData,
        raw: &[u8],
    ) -> Result<i64, SliceError<Calls, Files, Access>> {
        let document = Json::Object(entry.clone());
        let Ok(tokens) = count_batch_entry_tokens(&document, None, self.counter).await else {
            return Ok(row_estimate(raw));
        };
        Ok(i64::try_from(tokens).unwrap_or(i64::MAX))
    }
}
