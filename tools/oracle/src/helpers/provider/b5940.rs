use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Json;
use betterlite_provider::oci_common::base_url::get_oci_base_url;
use betterlite_provider::oci_common::credentials::{
    OciEnvValues, OciExplicitParams, resolve_oci_credentials,
};
use betterlite_provider::oci_common::schema::{
    OCI_JSON_TO_PYTHON_TYPES, enrich_cohere_description, resolve_oci_schema_anyof,
    resolve_oci_schema_refs, sanitize_oci_schema,
};
use betterlite_provider::oci_common::signing::sign_oci_request;
use betterlite_provider::oci_common::{Clock, OciError};
use chrono::{DateTime, Utc};
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use secrecy::ExposeSecret;
use serde::Deserialize;

use crate::helpers::HelperError;

const FIXED_SECONDS: i64 = 1_730_882_977;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Credentials {
        explicit: OciExplicitParams,
        env: OciEnvValues,
    },
    BaseUrl {
        explicit: OciExplicitParams,
        env: OciEnvValues,
        api_bases: Vec<Option<String>>,
    },
    Sign {
        explicit: OciExplicitParams,
        env: OciEnvValues,
        api_base: String,
        request: Json,
        headers: BTreeMap<String, String>,
    },
    Schema {
        schema: Json,
    },
    Description {
        description: String,
        schema: Json,
    },
}

fn oci_failure(error: OciError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "OciError".to_owned(),
        status: Some(error.status),
        error_type: None,
        param: None,
        code: None,
        message: error.message,
    })
}

fn input_failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn credentials(explicit: &OciExplicitParams, env: &OciEnvValues) -> serde_json::Value {
    let resolved = resolve_oci_credentials(explicit, env);
    serde_json::json!({
        "oci_region": resolved.oci_region,
        "oci_user": resolved.oci_user,
        "oci_fingerprint": resolved.oci_fingerprint,
        "oci_tenancy": resolved.oci_tenancy,
        "oci_key": resolved.oci_key.as_ref().map(ExposeSecret::expose_secret),
        "oci_key_file": resolved.oci_key_file,
        "oci_compartment_id": resolved.oci_compartment_id,
    })
}

fn base_urls(
    explicit: &OciExplicitParams,
    env: &OciEnvValues,
    api_bases: &[Option<String>],
) -> crate::helpers::HelperResult {
    let region = resolve_oci_credentials(explicit, env).oci_region;
    let mut urls = Vec::new();
    for api_base in api_bases {
        urls.push(get_oci_base_url(&region, api_base.as_deref()).map_err(oci_failure)?);
    }
    Ok(serde_json::json!(urls))
}

fn signature(
    explicit: &OciExplicitParams,
    env: &OciEnvValues,
    api_base: &str,
    request: &Json,
    headers: &BTreeMap<String, String>,
) -> crate::helpers::HelperResult {
    let credentials = resolve_oci_credentials(explicit, env);
    let mut header_map = HeaderMap::new();
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|source| input_failure(source.to_string()))?;
        let header_value =
            HeaderValue::from_str(value).map_err(|source| input_failure(source.to_string()))?;
        header_map.insert(header_name, header_value);
    }
    let now = DateTime::<Utc>::from_timestamp(FIXED_SECONDS, 0)
        .ok_or_else(|| input_failure("the fixed date is out of range".to_owned()))?;
    let body = sign_oci_request(
        &mut header_map,
        &credentials,
        request,
        api_base,
        &FixedClock(now),
    )
    .map_err(oci_failure)?;
    let body_text = String::from_utf8(body).map_err(|source| input_failure(source.to_string()))?;
    let mut signed = BTreeMap::new();
    for (name, value) in &header_map {
        let text = value
            .to_str()
            .map_err(|source| input_failure(source.to_string()))?;
        signed.insert(name.as_str().to_owned(), text.to_owned());
    }
    Ok(serde_json::json!({"headers": signed, "body": body_text}))
}

fn schema_output(schema: &Json) -> crate::helpers::HelperResult {
    Ok(serde_json::json!({
        "refs": resolve_oci_schema_refs(schema).map_err(oci_failure)?,
        "anyof": resolve_oci_schema_anyof(schema),
        "sanitized": sanitize_oci_schema(schema),
        "python_types": BTreeMap::from(OCI_JSON_TO_PYTHON_TYPES),
    }))
}

struct FixedClock(DateTime<Utc>);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Credentials { explicit, env } => Ok(credentials(explicit, env)),
            Input::BaseUrl {
                explicit,
                env,
                api_bases,
            } => base_urls(explicit, env, api_bases),
            Input::Sign {
                explicit,
                env,
                api_base,
                request,
                headers,
            } => signature(explicit, env, api_base, request, headers),
            Input::Schema { schema } => schema_output(schema),
            Input::Description {
                description,
                schema,
            } => {
                let object = schema
                    .as_object()
                    .ok_or_else(|| input_failure("schema must be an object".to_owned()))?;
                Ok(serde_json::json!(enrich_cohere_description(
                    description,
                    object
                )))
            }
        }
    }
}
