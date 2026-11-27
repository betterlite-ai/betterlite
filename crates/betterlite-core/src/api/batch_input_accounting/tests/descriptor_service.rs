use std::convert::Infallible;
use std::sync::atomic::AtomicBool;

use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_config::sections::batch_limit_descriptors::Settings;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;

use super::super::input::BatchInputAccountingInput;
use super::super::service::BatchInputAccounting;
use super::support::{CallsFailure, Harness, RecordedFetch, block_on, input_wire, row_usage};
use super::{CAP_ROW, MALFORMED_ROW, OTPM_CALLER};
use crate::api::api_boundaries::ApiLimits;
use crate::api::batch_limit_counters::rejections::PROJECT_OTPM_DESCRIPTOR_KEY as OTPM_KEY;
use crate::api::batch_limit_descriptors::BatchLimitDescriptors;
use crate::api::batch_limit_descriptors::descriptors::BatchLimitDescriptor;
use crate::api::batch_limit_descriptors::limiter::BatchLimiterSource;
use crate::api::model_credentials::CredentialSource;
use crate::api::request_destination::RequestData;

struct ScriptedLimiter {
    candidates: i64,
    floor: i64,
}

impl BatchLimiterSource for ScriptedLimiter {
    type Error = CallsFailure;

    const PROJECT_OTPM_DESCRIPTOR_KEY: &'static str = OTPM_KEY;

    fn descriptors_for(
        &self,
        _auth: &UserAPIKeyAuth,
        _data: &RequestData,
    ) -> Result<Vec<BatchLimitDescriptor>, CallsFailure> {
        Ok(Vec::new())
    }

    fn project_io_for(
        &self,
        _auth: &UserAPIKeyAuth,
        _model: &str,
        _out: &mut Vec<BatchLimitDescriptor>,
    ) {
    }

    fn candidate_count(&self, _body: &RequestData) -> i64 {
        self.candidates
    }

    fn output_floor(&self, min_configured_otpm_limit: Option<i64>) -> i64 {
        min_configured_otpm_limit.unwrap_or(self.floor)
    }
}

struct NoCredentials;

impl CredentialSource for NoCredentials {
    type Error = Infallible;

    fn credentials_with_provider(
        &self,
        _model_id: &str,
        _team_id: Option<&str>,
    ) -> Result<Option<JsonObject>, Infallible> {
        Ok(None)
    }
}

fn limit_descriptors(
    candidates: i64,
    floor: i64,
) -> BatchLimitDescriptors<ScriptedLimiter, NoCredentials> {
    BatchLimitDescriptors {
        settings: Settings::default(),
        limiter: ScriptedLimiter { candidates, floor },
        credentials: None::<NoCredentials>,
        limits: ApiLimits::default(),
        warned_unsupported_model_skip: AtomicBool::new(false),
    }
}

#[test]
fn estimates_and_fetch_params_run_through_the_descriptor_service() {
    let descriptors = limit_descriptors(2, 0);
    let harness = Harness::new(&format!("{CAP_ROW}\n{MALFORMED_ROW}"));
    harness.counter.script(&[3]);
    let service = BatchInputAccounting {
        calls: &descriptors,
        files: &harness.files,
        access: &harness.access,
        counter: &harness.counter,
        limits: harness.limits,
    };
    let input: BatchInputAccountingInput =
        serde_json::from_str(&input_wire("file-abc", Some(OTPM_CALLER), "[]"))
            .expect("the input decodes");
    let result = block_on(service.run(&input)).expect("the rows account");
    assert_eq!(result.total_tokens, 7);
    assert_eq!(result.output_tokens, 13);
    assert_eq!(result.request_count, 2);
    assert_eq!(
        result.per_model_usage,
        IndexMap::from([("gpt-4o".to_owned(), row_usage(3, 10, 1))])
    );
    let recorded = harness.files.recorded();
    match recorded.first().expect("the file was read") {
        RecordedFetch::Provider {
            file_id,
            params,
            caller,
        } => {
            assert_eq!(file_id, "file-abc");
            assert!(*caller);
            assert_eq!(
                params.get("custom_llm_provider"),
                Some(&Json::Text("openai".to_owned()))
            );
        }
        RecordedFetch::Managed { .. } => panic!("a plain file id is read from its provider"),
    }
}
