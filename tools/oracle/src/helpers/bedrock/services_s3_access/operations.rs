use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_bedrock::aws_settings::AUTHENTICATION_PARAMS;
use betterlite_bedrock::s3_access::decode::decode_managed_file_id;
use betterlite_bedrock::s3_access::handler::{AwsBase, BatchFiles, BedrockFilesHandler};
use betterlite_bedrock::s3_access::managed::{
    resolve_managed_s3_object, validate_configured_file_id,
};
use betterlite_bedrock::s3_access::models::{
    bedrock_model_id, invoke_route_model, strip_llm_routing_prefix,
};
use betterlite_bedrock::s3_access::settings::{
    S3Setting, S3SettingSources, SettingMap, SettingValue, resolve_s3_setting,
};
use betterlite_bedrock::s3_access::uri::parse_s3_uri;
use betterlite_bedrock::s3_access::{
    S3AccessError, S3AccessParams, TrustedModelCredentials, TrustedValue, configured_bucket_name,
    configured_bucket_names, frozen_mapping,
};
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};

use super::fixtures::{self, ResolutionFixture};
use super::{Input, Operation, WireValue};
use crate::helpers::{HelperError, HelperResult};

pub(super) fn run(input: &Input) -> HelperResult {
    match input.operation {
        Operation::ManagedFileIdDecode => Ok(decoded_values(&input.file_ids)),
        Operation::ConfiguredInputOutputBuckets => Ok(configured_buckets(input)),
        Operation::ConfiguredPrefixBoundary => Ok(prefix_boundary(input)),
        Operation::ForeignBucketRejected => Ok(foreign_bucket(input)),
        Operation::LegacyIdOptIn => Ok(legacy_id(input)),
        Operation::RequestCredentialsRejected => Ok(request_credentials(input)),
        Operation::KmsOwnerPrecedence => kms_owner(input),
        Operation::S3ModelHelpers => model_helpers(&input.models),
        Operation::PlainS3UriParser => Ok(parsed_uris(&input.uris)),
        Operation::FilesHandlerInitialization => handler_initialization(input),
    }
}

fn trusted_value(value: &WireValue) -> TrustedValue {
    match value {
        WireValue::Text(text) => TrustedValue::Text(text.clone()),
        WireValue::Boolean(flag) => TrustedValue::Boolean(*flag),
        WireValue::Null(()) => TrustedValue::Null,
        WireValue::Other(_) => TrustedValue::Other,
    }
}

fn snapshot(pairs: &[(String, WireValue)]) -> TrustedModelCredentials {
    frozen_mapping(
        pairs
            .iter()
            .map(|(name, value)| (name.clone(), trusted_value(value))),
    )
}

fn access_params(
    trusted: Option<&[(String, WireValue)]>,
    bucket: Option<&str>,
    output: Option<&str>,
) -> S3AccessParams {
    S3AccessParams {
        trusted: trusted.map(snapshot).unwrap_or_default(),
        bucket_name: bucket.map(str::to_owned),
        output_bucket_name: output.map(str::to_owned),
    }
}

fn decode_value(file_id: &str) -> Value {
    decode_managed_file_id(file_id).map_or_else(
        |_error| json!({"failed": true}),
        |value| json!({"value": value}),
    )
}

fn decoded_values(file_ids: &[String]) -> Value {
    json!({
        "values": file_ids.iter().map(|file_id| decode_value(file_id)).collect::<Vec<Value>>(),
    })
}

fn bucket_results(params: &S3AccessParams, environment: &BTreeMap<String, String>) -> Value {
    let single = configured_bucket_name(params, environment).map_or_else(
        |_error| json!({"failed": true}),
        |bucket| json!({"bucket": bucket}),
    );
    let multi = configured_bucket_names(params, environment).map_or_else(
        |_error| json!({"failed": true}),
        |buckets| json!({"buckets": buckets}),
    );
    json!({"single": single, "multi": multi})
}

fn configured_buckets(input: &Input) -> Value {
    let trusted = input.trusted.as_deref();
    let request = input.request_bucket_name.as_deref();
    let request_output = input.request_output_bucket_name.as_deref();
    let params = access_params(trusted, request, request_output);
    bucket_results(&params, &input.environment)
}

fn prefix_boundary(input: &Input) -> Value {
    let uri = input.s3_uri.as_deref().unwrap_or_default();
    match validate_configured_file_id(uri, &input.buckets, input.legacy) {
        Ok((bucket, object)) => json!({"bucket": bucket, "object": object}),
        Err(_error) => json!({"failed": true}),
    }
}

fn foreign_bucket(input: &Input) -> Value {
    let params = access_params(input.trusted.as_deref(), None, None);
    match resolve_managed_s3_object(
        input.file_id.as_deref().unwrap_or_default(),
        &params,
        &input.environment,
    ) {
        Ok((bucket, object)) => json!({"bucket": bucket, "object": object}),
        Err(error) => json!({"error": {"status": error.status_code()}}),
    }
}

fn legacy_id(input: &Input) -> Value {
    let params = access_params(input.trusted.as_deref(), None, None);
    let opt_in = params.trusted.allows_legacy();
    let buckets = input
        .bucket
        .as_deref()
        .map(str::to_owned)
        .into_iter()
        .collect::<Vec<String>>();
    let authorized = validate_configured_file_id(
        input.s3_uri.as_deref().unwrap_or_default(),
        &buckets,
        opt_in,
    )
    .map_or_else(
        |_error| json!({"failed": true}),
        |(bucket, object)| json!({"bucket": bucket, "object": object}),
    );
    json!({"opt_in": opt_in, "authorized": authorized})
}

fn request_credentials(input: &Input) -> Value {
    let bucket = input.request_bucket_name.as_deref();
    let output = input.request_output_bucket_name.as_deref();
    let environment = &input.environment;
    json!({
        "trusted": bucket_results(&access_params(input.trusted.as_deref(), bucket, output), environment),
        "plain": bucket_results(&access_params(None, bucket, output), environment),
        "request": bucket_results(&access_params(None, bucket, output), environment),
        "malformed": bucket_results(&access_params(input.malformed.as_deref(), None, None), environment),
    })
}

fn setting_value(value: &WireValue) -> SettingValue {
    match value {
        WireValue::Text(text) => SettingValue::Text(text.clone()),
        WireValue::Boolean(_) | WireValue::Null(()) | WireValue::Other(_) => SettingValue::Other,
    }
}

fn setting_map(source: &BTreeMap<String, WireValue>) -> SettingMap {
    source
        .iter()
        .map(|(name, value)| (name.clone(), setting_value(value)))
        .collect()
}

fn s3_failure(error: &S3AccessError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "S3AccessError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn kms_owner(input: &Input) -> HelperResult {
    let params = setting_map(&input.litellm_params);
    let optional = input
        .optional_params
        .as_ref()
        .map(setting_map)
        .unwrap_or_default();
    let lookup = fixtures::secrets();
    let sources = S3SettingSources {
        params: &params,
        optional_params: &optional,
        secrets: &lookup,
        environment: &input.environment,
    };
    let encryption = resolve_s3_setting(S3Setting::EncryptionKeyId, &sources)
        .map_err(|error| s3_failure(&error))?;
    let owner =
        resolve_s3_setting(S3Setting::BucketOwner, &sources).map_err(|error| s3_failure(&error))?;
    Ok(json!({
        "encryption_key_id": encryption.map(|secret| secret.expose_secret().to_owned()),
        "bucket_owner": owner.map(|secret| secret.expose_secret().to_owned()),
    }))
}

fn model_helpers(models: &[String]) -> HelperResult {
    let resolution = ResolutionFixture::default();
    let context = resolution.context();
    let shared = fixtures::credential_cache()?;
    let batch = BatchFiles {
        base: AwsBase {
            credential_cache: Arc::clone(&shared),
            authentication_params: &AUTHENTICATION_PARAMS,
        },
    };
    Ok(json!({
        "authentication_params": batch.base.authentication_params,
        "shared_cache": Arc::ptr_eq(&batch.base.credential_cache, &shared),
        "model_ids": models.iter().map(|model| bedrock_model_id(model)).collect::<Vec<String>>(),
        "stripped": models.iter().map(|model| strip_llm_routing_prefix(model, &context)).collect::<Vec<String>>(),
        "invoked": models.iter().map(|model| invoke_route_model(model, &context)).collect::<Vec<String>>(),
    }))
}

fn parsed_uris(uris: &[String]) -> Value {
    let values = uris
        .iter()
        .map(|uri| {
            parse_s3_uri(uri).map_or_else(
                |_error| json!({"failed": true}),
                |(bucket, key)| json!({"bucket": bucket, "key": key}),
            )
        })
        .collect::<Vec<Value>>();
    json!({"values": values})
}

fn handler_initialization(input: &Input) -> HelperResult {
    let handler = BedrockFilesHandler {
        base: AwsBase {
            credential_cache: fixtures::credential_cache()?,
            authentication_params: &AUTHENTICATION_PARAMS,
        },
        client: fixtures::client()?,
    };
    let params = access_params(input.trusted.as_deref(), None, None);
    let bucket = configured_bucket_name(&params, &input.environment).map_or_else(
        |_error| json!({"failed": true}),
        |bucket| json!({"bucket": bucket}),
    );
    Ok(json!({
        "authentication_params": handler.base.authentication_params,
        "client": true,
        "extract": input.file_ids.iter().map(|file_id| decode_value(file_id)).collect::<Vec<Value>>(),
        "bucket": bucket,
    }))
}
