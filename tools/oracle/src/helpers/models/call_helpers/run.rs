use std::collections::BTreeSet;

use betterlite_context::request_records::RequestRecordProjection;
use betterlite_models::call_helpers::arithmetic::{safe_divide, safe_divide_seconds};
use betterlite_models::call_helpers::drop_params::{
    drop_params_env_flag, drop_params_flag, normalize_drop_params,
};
use betterlite_models::call_helpers::expected_error::{ClientErrorFacts, is_expected_client_error};
use betterlite_models::call_helpers::finish_reason::map_finish_reason;
use betterlite_models::call_helpers::headers::{
    HeaderSource, TrustedHeaders, process_response_headers, provider_response_headers,
    proxy_server_request_headers, set_provider_response_headers,
};
use betterlite_models::call_helpers::metadata::{
    MetadataSlots, add_missing_spend_metadata, get_or_create_metadata_bucket,
    litellm_metadata_from_params, stamped_oauth_token,
};
use betterlite_models::call_helpers::model_name::reconstruct_model_name;
use betterlite_models::call_helpers::token_limit::coerce_token_limit;
use betterlite_models::decisions::Json;
use serde_json::{Value, json};

use super::input::{
    HeaderStep, Input, ModelNameInput, SafeDivideInput, SafeDivideSecondsInput, TrustedStep,
};

pub(super) fn run(input: &Input) -> Value {
    match input {
        Input::FinishReasons { values } => finish_reasons(values),
        Input::TokenLimits { values } => token_limits(values),
        Input::ExpectedClientError { facts } => expected_client_errors(facts),
        Input::MetadataBucket { steps } => metadata_buckets(steps),
        Input::TrustedFlags { steps } => trusted_flags(steps),
        Input::ModelReconstruction { values } => model_names(values),
        Input::ResponseHeaders { steps } => header_steps(steps),
        Input::UpstreamInternalHeader { upstream, trusted } => {
            let internal = TrustedHeaders {
                entries: trusted.clone(),
            };
            json!({
                "upstream": process_response_headers(HeaderSource::Upstream(upstream)),
                "trusted": process_response_headers(HeaderSource::Internal(&internal)),
            })
        }
        Input::SnapshotIsolation { record } => json!(record),
        Input::FilterInternal { record, additional } => {
            let names: BTreeSet<String> = additional.iter().cloned().collect();
            json!(RequestRecordProjection::filter_internal(
                record.clone(),
                &names
            ))
        }
        Input::DropEnvInvalid { values } => drop_env_flags(values),
        Input::DropRequestInvalid { values } => drop_request_flags(values),
        Input::SafeDivide { pairs } => safe_divides(pairs),
        Input::SafeDivideSeconds { pairs } => safe_divide_seconds_values(pairs),
        Input::RequestHeaders { steps } => Value::Array(
            steps
                .iter()
                .map(|params| json!(proxy_server_request_headers(params.as_ref())))
                .collect(),
        ),
    }
}

fn finish_reasons(values: &[String]) -> Value {
    Value::Array(
        values
            .iter()
            .map(|raw| {
                let mapped = map_finish_reason(raw);
                json!({"reason": mapped.reason.as_str(), "recognized": mapped.recognized})
            })
            .collect(),
    )
}

fn token_limits(values: &[Json]) -> Value {
    Value::Array(
        values
            .iter()
            .map(|value| json!(coerce_token_limit(Some(value))))
            .collect(),
    )
}

fn expected_client_errors(facts: &[Option<ClientErrorFacts>]) -> Value {
    Value::Array(
        facts
            .iter()
            .map(|given| json!(is_expected_client_error(given.as_ref())))
            .collect(),
    )
}

fn metadata_buckets(steps: &[MetadataSlots]) -> Value {
    Value::Array(
        steps
            .iter()
            .map(|step| {
                let mut slots = step.clone();
                let (name, bucket) = get_or_create_metadata_bucket(&mut slots);
                json!({"name": name.key(), "bucket": &*bucket})
            })
            .collect(),
    )
}

fn trusted_flags(steps: &[TrustedStep]) -> Value {
    Value::Array(steps.iter().map(trusted_flag).collect())
}

fn trusted_flag(step: &TrustedStep) -> Value {
    match step {
        TrustedStep::LitellmMetadata { litellm_params } => {
            json!(litellm_metadata_from_params(litellm_params.as_ref()))
        }
        TrustedStep::StampedOauthToken {
            metadata,
            litellm_params,
        } => json!(stamped_oauth_token(
            metadata.as_ref(),
            litellm_params.as_ref()
        )),
        TrustedStep::AddMissingSpendMetadata {
            litellm_metadata,
            metadata,
        } => {
            let mut merged = litellm_metadata.clone();
            add_missing_spend_metadata(&mut merged, metadata);
            json!(merged)
        }
    }
}

fn model_names(values: &[ModelNameInput]) -> Value {
    Value::Array(
        values
            .iter()
            .map(|entry| {
                json!(reconstruct_model_name(
                    &entry.model,
                    entry.provider.as_deref(),
                    entry.deployment.as_deref()
                ))
            })
            .collect(),
    )
}

fn header_steps(steps: &[HeaderStep]) -> Value {
    Value::Array(steps.iter().map(header_step).collect())
}

fn header_step(step: &HeaderStep) -> Value {
    match step {
        HeaderStep::Project { headers } => {
            json!(process_response_headers(HeaderSource::Upstream(headers)))
        }
        HeaderStep::SetHidden { headers, existing } => {
            let projection = set_provider_response_headers(existing, headers);
            json!({
                "headers": projection.headers,
                "additional_headers": projection.additional_headers,
            })
        }
        HeaderStep::GetHidden { hidden } => json!(provider_response_headers(hidden.as_ref())),
    }
}

fn drop_env_flags(values: &[Option<String>]) -> Value {
    Value::Array(
        values
            .iter()
            .map(|configured| {
                let flag = drop_params_env_flag(configured.as_deref());
                json!({"enabled": flag.enabled, "warned": flag.warned})
            })
            .collect(),
    )
}

fn drop_request_flags(values: &[Json]) -> Value {
    let mut flags = Vec::with_capacity(values.len());
    for value in values {
        let flag = drop_params_flag(Some(value));
        flags.push(json!({
            "normalized": normalize_drop_params(Some(value)),
            "enabled": flag.enabled,
            "warned": flag.warned,
        }));
    }
    Value::Array(flags)
}

fn safe_divides(pairs: &[SafeDivideInput]) -> Value {
    Value::Array(
        pairs
            .iter()
            .map(|pair| {
                json!(
                    safe_divide(pair.numerator, pair.denominator, pair.default)
                        .normalize()
                        .to_string()
                )
            })
            .collect(),
    )
}

fn safe_divide_seconds_values(pairs: &[SafeDivideSecondsInput]) -> Value {
    Value::Array(
        pairs
            .iter()
            .map(|pair| {
                json!(
                    safe_divide_seconds(pair.seconds, pair.denominator, pair.default)
                        .map(|value| value.normalize().to_string())
                )
            })
            .collect(),
    )
}
