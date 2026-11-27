use std::collections::BTreeMap;
use std::time::SystemTime;

use betterlite_core::api::polly::speech::credentials::{
    self, CredentialResolver, CredentialSources, ResolvedCredentials,
};
use betterlite_core::api::polly::speech::endpoint::complete_url;
use betterlite_core::api::polly::speech::error::SpeechError;
use betterlite_core::api::polly::speech::mapping::map_openai_params;
use betterlite_core::api::polly::speech::region::RegionEnvironment;
use betterlite_core::api::polly::speech::request::{
    Dispatch, SignedRequest, TextToSpeechRequest, dispatch, transform_request,
};
use betterlite_core::api::speech_call::operations::SpeechMappingOptions;
use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::openai_base_config::Object;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

const MODEL: &str = "aws_polly/neural";
const EXAMPLE_KEY: &str = "AKIDEXAMPLE";
const EXAMPLE_SECRET: &str = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";

// every call sends the keys its function needs; the others stay at their defaults
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    function: String,
    model: Option<String>,
    text: Option<String>,
    voice: Option<SpeechVoice>,
    #[serde(default)]
    optional_params: Object,
    #[serde(default)]
    litellm_params: Object,
    #[serde(default)]
    kwargs: Object,
    kwargs_region: Option<String>,
    api_base: Option<String>,
    api_key: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let answers: Vec<Value> = input.calls.iter().map(answer).collect();
        Ok(json!({ "calls": answers }))
    }
}

fn answer(call: &Call) -> Value {
    match call.function.as_str() {
        "dispatch" => dispatch_answer(Dispatch {
            voice: call.voice.as_ref(),
            optional_params: call.optional_params.clone(),
            litellm_params: call.litellm_params.clone(),
            kwargs: &region_kwargs(call.kwargs_region.as_deref()),
            api_base: call.api_base.as_deref(),
            api_key: call.api_key.as_deref(),
            environment: region_environment(&call.environment),
        }),
        "map_params" => map_answer(
            call.model.as_deref().unwrap_or(MODEL),
            call.voice.as_ref(),
            &call.optional_params,
            &call.kwargs,
        ),
        "url" => url_answer(call.api_base.as_deref(), &call.litellm_params),
        "request" => request_answer(
            call.text.as_deref().unwrap_or_default(),
            call.voice.as_ref(),
            &call.optional_params,
            &call.litellm_params,
        ),
        "credential_arguments" => credential_answer(&call.litellm_params),
        _ => json!({ "ok": false }),
    }
}

// dispatch reads the kwargs region and folds it into the stream of arguments the handler takes
fn region_kwargs(region: Option<&str>) -> Object {
    let mut kwargs = Object::new();
    if let Some(name) = region {
        kwargs.insert(String::from("aws_region_name"), json!(name));
    }
    kwargs
}

fn map_answer(
    model: &str,
    voice: Option<&SpeechVoice>,
    optional_params: &Object,
    kwargs: &Object,
) -> Value {
    let options = SpeechMappingOptions {
        voice: voice.cloned(),
        drop_params: false,
        kwargs: kwargs.clone(),
    };
    let (mapped_voice, params) = map_openai_params(model, optional_params, &options);
    json!({ "ok": true, "voice": mapped_voice, "params": params })
}

// get_complete_url takes the base as an argument and reads only the region off litellm_params
fn url_answer(api_base: Option<&str>, litellm_params: &Object) -> Value {
    let region = litellm_params
        .get("aws_region_name")
        .and_then(Value::as_str);
    let url = complete_url(api_base, region);
    json!({ "ok": true, "url": url })
}

// the handler receives the mapped voice, the untouched options and the rewritten litellm_params
fn dispatch_answer(call: Dispatch<'_>) -> Value {
    let Ok(dispatched) = dispatch(call) else {
        return json!({ "ok": false });
    };
    json!({ "ok": true, "voice": dispatched.voice, "optional_params": dispatched.optional_params, "litellm_params": dispatched.litellm_params })
}

fn request_answer(
    text: &str,
    voice: Option<&SpeechVoice>,
    optional_params: &Object,
    litellm_params: &Object,
) -> Value {
    transform_request(
        &SignedRequest {
            input: text,
            voice: voice_name(voice),
            optional_params,
            litellm_params,
            now: SystemTime::now(),
        },
        &FixedResolver,
    )
    .map_or_else(|_| json!({ "ok": false }), |signed| signed_answer(&signed))
}

fn signed_answer(signed: &TextToSpeechRequest) -> Value {
    let Some(body) = signed.data.ssml_body.as_deref() else {
        return json!({ "ok": false });
    };
    let Some(headers) = signed.data.headers.as_ref() else {
        return json!({ "ok": false });
    };
    let Some(fields) = signed_fields(headers) else {
        return json!({ "ok": false });
    };
    json!({
        "ok": true, "url": signed.url, "dict_body_absent": signed.data.dict_body.is_none(), "body": body,
        "signed": { "scheme": fields.scheme, "credential": fields.credential, "scope": fields.scope, "signed_headers": fields.signed_headers, "signature_ok": fields.signature_ok, "date_ok": fields.date_ok, "header_names": fields.header_names, "content_type": fields.content_type, "security_token": fields.security_token },
    })
}

fn credential_answer(litellm_params: &Object) -> Value {
    let Ok(sources) = credentials::sources(litellm_params) else {
        return json!({ "ok": false });
    };
    json!({
        "ok": true, "arguments": {
            "aws_access_key_id": sources.aws_access_key_id,
            "aws_secret_access_key": sources.aws_secret_access_key,
            "aws_session_token": sources.aws_session_token,
            "aws_region_name": sources.aws_region_name,
            "aws_session_name": sources.aws_session_name,
            "aws_profile_name": sources.aws_profile_name,
            "aws_role_name": sources.aws_role_name,
            "aws_web_identity_token": sources.aws_web_identity_token,
            "aws_sts_endpoint": sources.aws_sts_endpoint,
            "aws_external_id": sources.aws_external_id,
        }
    })
}

// the signing region, the credential key, the token and the payload shape; the signature and date values differ by clock
struct SignedFields {
    scheme: String,
    credential: String,
    scope: [String; 3],
    signed_headers: Vec<String>,
    signature_ok: bool,
    date_ok: bool,
    header_names: Vec<String>,
    content_type: Option<String>,
    security_token: Option<String>,
}

fn signed_fields(headers: &IndexMap<String, String>) -> Option<SignedFields> {
    let lowered: BTreeMap<String, String> = headers
        .iter()
        .map(|(name, value)| (name.to_lowercase(), value.clone()))
        .collect();
    let (scheme, rest) = lowered.get("authorization")?.split_once(' ')?;
    let mut credential = None;
    let mut signed_headers = None;
    let mut signature = None;
    for field in rest.split(", ") {
        let (name, value) = field.split_once('=')?;
        match name {
            "Credential" => credential = Some(value),
            "SignedHeaders" => signed_headers = Some(value),
            "Signature" => signature = Some(value),
            _ => {}
        }
    }
    let mut parts = credential?.split('/');
    let key = parts.next()?;
    let _date = parts.next()?;
    let region = parts.next()?;
    let service = parts.next()?;
    let terminator = parts.next()?;
    Some(SignedFields {
        scheme: scheme.to_owned(),
        credential: key.to_owned(),
        scope: [region.to_owned(), service.to_owned(), terminator.to_owned()],
        signed_headers: signed_headers?.split(';').map(str::to_owned).collect(),
        signature_ok: signature_ok(signature?),
        date_ok: date_ok(lowered.get("x-amz-date")?),
        header_names: lowered.keys().cloned().collect(),
        content_type: lowered.get("content-type").cloned(),
        security_token: lowered.get("x-amz-security-token").cloned(),
    })
}

// botocore signs with a lowercase hex digest and an ISO basic UTC date
fn signature_ok(signature: &str) -> bool {
    signature.len() == 64
        && signature
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn date_ok(date: &str) -> bool {
    let Some(stem) = date.strip_suffix('Z') else {
        return false;
    };
    let Some((day, clock)) = stem.split_once('T') else {
        return false;
    };
    day.len() == 8
        && clock.len() == 6
        && day.bytes().all(|byte| byte.is_ascii_digit())
        && clock.bytes().all(|byte| byte.is_ascii_digit())
}

fn region_environment(environment: &BTreeMap<String, String>) -> RegionEnvironment<'_> {
    RegionEnvironment {
        region_name: named(environment, "AWS_REGION_NAME"),
        region: named(environment, "AWS_REGION"),
    }
}

// an empty variable counts as unset, as docs/compatibility.md says
fn named<'env>(environment: &'env BTreeMap<String, String>, name: &str) -> Option<&'env str> {
    environment
        .get(name)
        .map(String::as_str)
        .filter(|value| !value.is_empty())
}

fn voice_name(voice: Option<&SpeechVoice>) -> Option<&str> {
    match voice {
        Some(SpeechVoice::Name(name)) => Some(name.as_str()),
        Some(SpeechVoice::Options(_)) | None => None,
    }
}

// the case supplies the explicit credentials LiteLLM's get_credentials would hand back unchanged
struct FixedResolver;

impl CredentialResolver for FixedResolver {
    fn resolve(&self, sources: &CredentialSources<'_>) -> Result<ResolvedCredentials, SpeechError> {
        let key = sources.aws_access_key_id.unwrap_or(EXAMPLE_KEY);
        let secret = sources.aws_secret_access_key.unwrap_or(EXAMPLE_SECRET);
        Ok(ResolvedCredentials {
            access_key_id: SecretString::from(key.to_owned()),
            secret_access_key: SecretString::from(secret.to_owned()),
            session_token: sources
                .aws_session_token
                .map(|token| SecretString::from(token.to_owned())),
        })
    }
}

mod tests;
