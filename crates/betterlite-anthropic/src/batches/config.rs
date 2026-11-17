use betterlite_http::url_path::encode_segment;
use betterlite_models::decisions::Json;
use betterlite_openai::batches::records::{BatchObject, BatchObjectKind, RequestCounts};
use http::header::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use crate::credentials::{ANTHROPIC_VERSION, AnthropicSettings, get_auth_header};

use super::decode::{
    BatchBody, DecodeError, DecodedBody, LifecycleBatch, decode_anthropic_batch_result,
};
use super::error::BatchesError;
use super::timestamps::{Clock, parse_timestamp};
use super::{
    BATCHES_BETA_HEADER, BATCHES_ROUTE, COMPLETION_WINDOW, DEFAULT_INPUT_FILE_ID, ENDPOINT,
};

// The pieces LiteLLM's validate_environment resolves before it builds the auth header
pub struct Environment<'call> {
    pub api_key: Option<&'call SecretString>,
    pub api_base: Option<&'call str>,
    pub params_api_base: Option<&'call str>,
}

#[derive(Debug)]
pub struct AnthropicBatchesConfig {
    pub settings: AnthropicSettings,
}

// LiteLLM's BaseBatchesConfig surface, with the creation placeholders it leaves unimplemented
pub trait BatchesConfig {
    fn validate_environment(
        &self,
        headers: &mut HeaderMap,
        environment: Environment<'_>,
    ) -> Result<(), BatchesError>;

    fn get_complete_batch_url(&self, api_base: Option<&str>) -> String;

    fn transform_create_batch_request(&self) -> Result<Vec<u8>, BatchesError>;

    // ast-grep-ignore: no-long-names
    fn transform_create_batch_response(&self) -> Result<BatchObject, BatchesError>;

    fn get_retrieve_batch_url(
        &self,
        api_base: Option<&str>,
        batch_id: &str,
    ) -> Result<String, BatchesError>;

    // ast-grep-ignore: no-long-names
    fn transform_retrieve_batch_request(&self) -> IndexMap<String, Json>;

    // ast-grep-ignore: no-long-names
    fn transform_retrieve_batch_response(
        &self,
        body: &[u8],
        clock: &Clock,
    ) -> Result<BatchObject, DecodeError>;
}

impl BatchesConfig for AnthropicBatchesConfig {
    fn validate_environment(
        &self,
        headers: &mut HeaderMap,
        environment: Environment<'_>,
    ) -> Result<(), BatchesError> {
        let api_base = environment.api_base.or(environment.params_api_base);
        let auth = get_auth_header(environment.api_key, api_base, false, &self.settings)
            .ok_or(BatchesError::MissingEnvironmentKey)?;
        if !headers.contains_key("anthropic-beta") {
            headers.insert(
                HeaderName::from_static("anthropic-beta"),
                HeaderValue::from_static(BATCHES_BETA_HEADER),
            );
        }
        headers.insert(
            HeaderName::from_static("accept"),
            HeaderValue::from_static("application/json"),
        );
        headers.insert(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("application/json"),
        );
        headers.insert(
            HeaderName::from_static("anthropic-version"),
            HeaderValue::from_static(ANTHROPIC_VERSION),
        );
        let mut value = HeaderValue::from_str(auth.value.expose_secret()).map_err(|_invalid| {
            BatchesError::InvalidHeader {
                name: auth.name.to_owned(),
            }
        })?;
        value.set_sensitive(true);
        headers.insert(HeaderName::from_static(auth.name), value);
        Ok(())
    }

    fn get_complete_batch_url(&self, api_base: Option<&str>) -> String {
        let base = api_base
            .filter(|value| !value.is_empty())
            .unwrap_or(&self.settings.api_base);
        if base.ends_with(BATCHES_ROUTE) {
            return base.to_owned();
        }
        format!("{}{BATCHES_ROUTE}", base.trim_end_matches('/'))
    }

    fn transform_create_batch_request(&self) -> Result<Vec<u8>, BatchesError> {
        Err(BatchesError::NotImplemented)
    }

    // ast-grep-ignore: no-long-names
    fn transform_create_batch_response(&self) -> Result<BatchObject, BatchesError> {
        Err(BatchesError::NotImplemented)
    }

    fn get_retrieve_batch_url(
        &self,
        api_base: Option<&str>,
        batch_id: &str,
    ) -> Result<String, BatchesError> {
        let base = api_base
            .filter(|value| !value.is_empty())
            .unwrap_or(&self.settings.api_base);
        let encoded = encode_segment(batch_id, "batch_id")?;
        Ok(format!(
            "{}{BATCHES_ROUTE}/{encoded}",
            base.trim_end_matches('/')
        ))
    }

    // the empty map LiteLLM returns is Default's value too, so no test can tell its mutants apart
    #[cfg_attr(test, mutants::skip)]
    // ast-grep-ignore: no-long-names
    fn transform_retrieve_batch_request(&self) -> IndexMap<String, Json> {
        IndexMap::new()
    }

    // ast-grep-ignore: no-long-names
    fn transform_retrieve_batch_response(
        &self,
        body: &[u8],
        clock: &Clock,
    ) -> Result<BatchObject, DecodeError> {
        let DecodedBody::Lifecycle(batch) =
            decode_anthropic_batch_result(BatchBody::Lifecycle, body)?
        else {
            return Err(DecodeError::Shape {
                detail: "the batch body must be a lifecycle object",
            });
        };
        project_batch(&batch, clock)
    }
}

// LiteLLM fills each status mark only from the timestamp of the status that owns it
struct BatchMarks {
    cancelled: Option<i64>,
    cancelling: Option<i64>,
    completed: Option<i64>,
    in_progress: Option<i64>,
}

fn status_marks(
    batch: &LifecycleBatch,
    clock: &Clock,
    created: Option<i64>,
    ended: Option<i64>,
) -> BatchMarks {
    let raw_status = batch.raw_status.as_deref();
    if raw_status == Some("canceling") {
        return BatchMarks {
            cancelled: ended.filter(|value| *value != 0),
            cancelling: parse_timestamp(&batch.cancel_initiated_at, clock.zone),
            completed: None,
            in_progress: None,
        };
    }
    BatchMarks {
        cancelled: None,
        cancelling: None,
        completed: if raw_status == Some("ended") {
            ended
        } else {
            None
        },
        in_progress: if raw_status == Some("in_progress") {
            created
        } else {
            None
        },
    }
}

fn project_batch(batch: &LifecycleBatch, clock: &Clock) -> Result<BatchObject, DecodeError> {
    let created = parse_timestamp(&batch.created_at, clock.zone);
    let ended = parse_timestamp(&batch.ended_at, clock.zone);
    let marks = status_marks(batch, clock, created, ended);
    Ok(BatchObject {
        id: batch.id.clone(),
        completion_window: COMPLETION_WINDOW.to_owned(),
        created_at: created.filter(|value| *value != 0).unwrap_or(clock.now),
        endpoint: ENDPOINT.to_owned(),
        input_file_id: DEFAULT_INPUT_FILE_ID.to_owned(),
        object: BatchObjectKind::Batch,
        status: batch.status,
        cancelled_at: marks.cancelled,
        cancelling_at: marks.cancelling,
        completed_at: marks.completed,
        error_file_id: None,
        errors: None,
        expired_at: parse_timestamp(&batch.archived_at, clock.zone).filter(|value| *value != 0),
        expires_at: parse_timestamp(&batch.expires_at, clock.zone),
        failed_at: None,
        finalizing_at: None,
        in_progress_at: marks.in_progress,
        metadata: Some(IndexMap::new()),
        model: None,
        output_file_id: Some(batch.id.clone()),
        request_counts: Some(RequestCounts {
            completed: batch.counts.succeeded,
            failed: batch.counts.errored,
            total: batch.counts.total()?,
        }),
        usage: None,
        provider_fields: <_>::default(),
    })
}
