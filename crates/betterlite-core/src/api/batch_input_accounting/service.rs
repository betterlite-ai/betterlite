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
use super::rows::{RowInput, countable_entry, entry_model};
use super::totals::{Totals, min_configured_otpm_limit};
use super::{DecodedBatchInputLine, decode_batch_input_accounting};
use crate::api::api_boundaries::ApiLimits;
use crate::api::batch_managed_records::MANAGED_FILE_ID_PREFIX;
use crate::api::batch_output_usage::tokens::{
    BatchTokenCounter, count_batch_entry_tokens, estimate_batch_entry_tokens,
};
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

// What one pass over the file carries from row to row
struct Pass {
    min: Option<i64>,
    totals: Totals,
    models: IndexSet<String>,
}

// _PROXY_BatchRateLimiter.count_input_file_usage over an already fetched file
pub struct BatchInputAccounting<'slice, Calls, Files, Access> {
    pub calls: &'slice Calls,
    pub files: &'slice Files,
    pub access: &'slice Access,
    pub counter: &'slice (dyn BatchTokenCounter + 'static),
    pub limits: ApiLimits,
}

impl<Calls, Files, Access> BatchInputAccounting<'_, Calls, Files, Access>
where
    Calls: BatchAccountingCalls,
    Files: BatchFileSource,
    Access: BatchModelAccess,
{
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
        let mut pass = Pass {
            min,
            totals: Totals::default(),
            models: IndexSet::new(),
        };
        for line in &lines {
            self.account_line(line, &mut pass).await?;
        }
        if let Some(principal) = &caller {
            self.enforce_models(principal, &pass.models, loaded.target_models.as_deref())
                .await?;
        }
        Ok(BatchInputAccountingResult {
            total_tokens: pass.totals.total_tokens,
            request_count: pass.totals.request_count,
            output_tokens: pass.totals.output_tokens,
            per_model_usage: pass.totals.per_model,
        })
    }

    // Managed files go through the caller-aware hook; every other file id is read from its provider
    async fn load_content(
        &self,
        input: &BatchInputAccountingInput,
        caller: Option<&UserAPIKeyAuth>,
    ) -> Result<LoadedBatch, SliceError<Calls, Files, Access>> {
        let managed = is_base64_encoded_unified_id(&input.file_id, MANAGED_FILE_ID_PREFIX);
        let (fetched, target_models) = if let (Some(decoded), Some(principal)) = (managed, caller) {
            let names = target_model_names(&decoded);
            let request = BatchFileRequest::Managed {
                file_id: &input.file_id,
                caller: principal,
            };
            let fetched = self
                .files
                .fetch(request)
                .await
                .map_err(BatchInputAccountingError::Files)?;
            (fetched, Some(names))
        } else {
            let params = self
                .calls
                .resolve_fetch_params(
                    &input.file_id,
                    input.custom_llm_provider.slug(),
                    &input.data,
                )
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
        };
        Ok(LoadedBatch {
            bytes: Self::fetched_bytes(fetched, &input.file_id)?,
            target_models: target_models.filter(|names| !names.is_empty()),
        })
    }

    // The fetched bytes, or the failure LiteLLM raises for the other fetch outcomes
    fn fetched_bytes(
        fetched: BatchFileFetch,
        file_id: &str,
    ) -> Result<Vec<u8>, SliceError<Calls, Files, Access>> {
        match fetched {
            BatchFileFetch::Content(bytes) => Ok(bytes),
            BatchFileFetch::NotBytes { kind } => Err(BatchInputAccountingError::ContentNotBytes {
                file_id: file_id.to_owned(),
                kind,
            }),
            BatchFileFetch::ManagedUnavailable(reason) => {
                Err(BatchInputAccountingError::Managed(reason))
            }
            BatchFileFetch::Http(http) => Err(BatchInputAccountingError::Http(http)),
        }
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
        pass: &mut Pass,
    ) -> Result<(), SliceError<Calls, Files, Access>> {
        pass.totals.request_count = pass.totals.request_count.saturating_add(1);
        match line {
            DecodedBatchInputLine::Entry { entry, raw } => {
                self.account_entry(entry, raw, pass).await
            }
            DecodedBatchInputLine::Fallback { raw } => {
                let output = self.calls.output_floor(pass.min);
                pass.totals.add(
                    None,
                    i64::try_from(estimate_batch_entry_tokens(raw)).unwrap_or(i64::MAX),
                    output,
                );
                Ok(())
            }
        }
    }

    async fn account_entry(
        &self,
        entry: &RequestData,
        raw: &[u8],
        pass: &mut Pass,
    ) -> Result<(), SliceError<Calls, Files, Access>> {
        let model = entry_model(entry);
        if let Some(name) = model {
            pass.models.insert(name.to_owned());
        }
        let output = self
            .calls
            .estimate_output_tokens(entry, pass.min)
            .map_err(BatchInputAccountingError::Calls)?;
        let total = match countable_entry(entry) {
            RowInput::Zero => 0,
            RowInput::Unmeasurable => {
                i64::try_from(estimate_batch_entry_tokens(raw)).unwrap_or(i64::MAX)
            }
            RowInput::Measured(measured) => self.count_measured(&measured, raw).await?,
        };
        pass.totals.add(model, total, output);
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
            return Ok(i64::try_from(estimate_batch_entry_tokens(raw)).unwrap_or(i64::MAX));
        };
        Ok(i64::try_from(tokens).unwrap_or(i64::MAX))
    }
}
