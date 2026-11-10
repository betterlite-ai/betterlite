use std::collections::BTreeMap;
use std::sync::Arc;

use base64::Engine as _;
use base64::alphabet::STANDARD;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use betterlite_config::sections::bedrock_services_s3_access::{
    BUCKET_NAME_ENV, BUCKET_OWNER_ENV, ENCRYPTION_KEY_ID_ENV, OUTPUT_BUCKET_NAME_ENV,
};
use betterlite_context::cloud_storage::policy::{
    LegacyCloudSetting, TrustedCloudCredentials, allow_legacy,
};
use betterlite_context::cloud_storage::{
    BEDROCK_MANAGED_S3_PREFIXES, CloudStorageError, decode_cloud_object_reference,
};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::resolution::{ResolutionContext, ResolutionRequest, resolve};
use betterlite_openai::error::{ErrorRequest, ErrorResponse};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::error::SecretLookupError;
use secrecy::{ExposeSecret as _, SecretString};

use crate::aws_settings::credential_cache::CredentialCache;
use crate::bedrock_settings::{BedrockError, ErrorInput, ErrorReply, HeaderError, ResponseBody};

pub const S3_BUCKET_REQUIRED_ERROR: &str = "S3 bucket_name is required. Set 's3_bucket_name' in proxy config or AWS_S3_BUCKET_NAME for Bedrock file content retrieval.";
pub const MANAGED_FILE_ID_ERROR: &str = "file_id must be a managed LiteLLM S3 file id";
pub const REJECTED_FILE_ID_REQUEST_URL: &str = "https://litellm.ai";
pub const MANAGED_FILE_ID_PREFIX: &str = "litellm_proxy";
pub const MANAGED_FILE_ID_MARKER: &str = "llm_output_file_id,";

const S3_SCHEME: &str = "s3://";
const BEDROCK_ROUTING_PREFIX: &str = "bedrock/";
const INVOKE_ROUTE_PREFIX: &str = "invoke/";
const BUCKET_NAME_PARAM: &str = "s3_bucket_name";
const OUTPUT_BUCKET_NAME_PARAM: &str = "s3_output_bucket_name";
const ENCRYPTION_KEY_ID_PARAM: &str = "s3_encryption_key_id";
const BUCKET_OWNER_PARAM: &str = "s3_bucket_owner";
const LEGACY_SETTING: &str = "allow_legacy_cloud_file_ids";

// Python's b64decode drops every byte outside the alphabet, takes trailing padding and keeps non-zero trailing bits
const DECODER: GeneralPurpose = GeneralPurpose::new(
    &STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_allow_trailing_bits(true)
        .with_decode_padding_mode(DecodePaddingMode::RequireCanonical),
);

#[derive(Debug, thiserror::Error)]
pub enum S3AccessError {
    #[error("file_id must be a managed LiteLLM S3 file id")]
    ManagedFileId,
    #[error(
        "S3 bucket_name is required. Set 's3_bucket_name' in proxy config or AWS_S3_BUCKET_NAME for Bedrock file content retrieval."
    )]
    BucketRequired,
    #[error("s3_bucket_name: Input should be a valid string")]
    BucketNameType,
    #[error("s3_output_bucket_name: Input should be a valid string")]
    OutputBucketNameType,
    #[error("Invalid S3 URI format: {uri}")]
    InvalidS3Uri { uri: String },
    #[error(transparent)]
    Storage(#[from] CloudStorageError),
    #[error(transparent)]
    Secret(#[from] SecretLookupError),
    #[error(transparent)]
    Header(#[from] HeaderError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustedValue {
    Text(String),
    Boolean(bool),
    Null,
    Other,
}

// The immutable copy of _litellm_internal_model_credentials; the private map is only built by frozen_mapping.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustedModelCredentials {
    values: BTreeMap<String, TrustedValue>,
}

#[must_use]
pub fn frozen_mapping<I>(items: I) -> TrustedModelCredentials
where
    I: IntoIterator<Item = (String, TrustedValue)>,
{
    let mut values = BTreeMap::new();
    for (name, value) in items {
        values.insert(name, value);
    }
    TrustedModelCredentials { values }
}

impl TrustedModelCredentials {
    fn text(&self, name: &str, wrong_type: S3AccessError) -> Result<Option<String>, S3AccessError> {
        match self.values.get(name) {
            None | Some(TrustedValue::Null) => Ok(None),
            Some(TrustedValue::Text(value)) => Ok(Some(value.clone())),
            Some(TrustedValue::Boolean(_) | TrustedValue::Other) => Err(wrong_type),
        }
    }

    fn bucket_fields(&self) -> Result<(Option<String>, Option<String>), S3AccessError> {
        let bucket = self.text(BUCKET_NAME_PARAM, S3AccessError::BucketNameType)?;
        let output = self.text(
            OUTPUT_BUCKET_NAME_PARAM,
            S3AccessError::OutputBucketNameType,
        )?;
        Ok((bucket, output))
    }

    fn legacy_setting(&self) -> LegacyCloudSetting {
        match self.values.get(LEGACY_SETTING) {
            Some(TrustedValue::Boolean(flag)) => LegacyCloudSetting::Boolean(*flag),
            Some(TrustedValue::Text(value)) => LegacyCloudSetting::Text(value.clone()),
            Some(TrustedValue::Null | TrustedValue::Other) | None => LegacyCloudSetting::Other,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct S3AccessParams {
    pub trusted: Option<TrustedModelCredentials>,
    // a deployment or request bucket name carried in litellm_params never authorizes a file id on its own
    pub bucket_name: Option<String>,
    pub output_bucket_name: Option<String>,
}

fn bucket_or_environment(
    trusted: Option<String>,
    environment: &BTreeMap<String, String>,
    variable: &str,
) -> Option<String> {
    trusted.filter(|bucket| !bucket.is_empty()).or_else(|| {
        environment
            .get(variable)
            .filter(|bucket| !bucket.is_empty())
            .cloned()
    })
}

pub fn configured_bucket_name(
    params: &S3AccessParams,
    environment: &BTreeMap<String, String>,
) -> Result<String, S3AccessError> {
    let trusted = params
        .trusted
        .as_ref()
        .map(TrustedModelCredentials::bucket_fields)
        .transpose()?
        .and_then(|(input, _output)| input);
    bucket_or_environment(trusted, environment, BUCKET_NAME_ENV)
        .ok_or(S3AccessError::BucketRequired)
}

pub fn configured_bucket_names(
    params: &S3AccessParams,
    environment: &BTreeMap<String, String>,
) -> Result<Vec<String>, S3AccessError> {
    let (input, output) = params
        .trusted
        .as_ref()
        .map(TrustedModelCredentials::bucket_fields)
        .transpose()?
        .unwrap_or_default();
    let mut buckets = Vec::new();
    for bucket in [
        bucket_or_environment(input, environment, BUCKET_NAME_ENV),
        bucket_or_environment(output, environment, OUTPUT_BUCKET_NAME_ENV),
    ]
    .into_iter()
    .flatten()
    {
        if !buckets.contains(&bucket) {
            buckets.push(bucket);
        }
    }
    if buckets.is_empty() {
        return Err(S3AccessError::BucketRequired);
    }
    Ok(buckets)
}

#[must_use]
pub fn allow_legacy_cloud_file_ids(params: &S3AccessParams) -> bool {
    let credentials = params
        .trusted
        .as_ref()
        .map(|snapshot| TrustedCloudCredentials::from(snapshot.legacy_setting()));
    allow_legacy(credentials.as_ref())
}

fn permissive_base64_decode(input: &[u8]) -> Option<Vec<u8>> {
    let mut symbols = Vec::with_capacity(input.len());
    let mut trailing_padding = 0_usize;
    for &byte in input {
        let symbol = match byte {
            b'-' => b'+',
            b'_' => b'/',
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/' => byte,
            b'=' => {
                trailing_padding = trailing_padding.saturating_add(1);
                continue;
            }
            _ => continue,
        };
        symbols.push(symbol);
        trailing_padding = 0;
    }
    let enough_padding = match symbols.len() & 3 {
        0 => true,
        2 => trailing_padding >= 2,
        3 => trailing_padding >= 1,
        _ => false,
    };
    if !enough_padding {
        return None;
    }
    while !symbols.len().is_multiple_of(4) {
        symbols.push(b'=');
    }
    DECODER.decode(&symbols).ok()
}

fn decode_base64_text(file_id: &str) -> Option<String> {
    if !file_id.is_ascii() {
        return None;
    }
    // Python pads -len(file_id) % 4 characters before decoding, which also satisfies its trailing-padding rule
    let padding = 4_usize.wrapping_sub(file_id.len() & 3) & 3;
    let mut padded = String::with_capacity(file_id.len().saturating_add(padding));
    padded.push_str(file_id);
    for _ in 0..padding {
        padded.push('=');
    }
    let decoded = permissive_base64_decode(padded.as_bytes())?;
    String::from_utf8(decoded).ok()
}

fn decode_managed_text(file_id: &str) -> Option<String> {
    let decoded = decode_base64_text(file_id)?;
    if !decoded.starts_with(MANAGED_FILE_ID_PREFIX) {
        return None;
    }
    let rest = decoded.split_once(MANAGED_FILE_ID_MARKER)?.1;
    let marker_at = rest.find(MANAGED_FILE_ID_MARKER).unwrap_or(rest.len());
    let semicolon_at = rest.find(';').unwrap_or(rest.len());
    rest.get(..marker_at.min(semicolon_at)).map(str::to_owned)
}

pub fn decode_managed_file_id(file_id: &str) -> Result<String, S3AccessError> {
    if let Some(decoded) = decode_managed_text(file_id) {
        return Ok(decoded);
    }
    if file_id.starts_with(S3_SCHEME) {
        return Ok(file_id.to_owned());
    }
    Err(S3AccessError::ManagedFileId)
}

pub fn validate_configured_file_id(
    s3_uri: &str,
    configured_bucket_names: &[String],
    allow_legacy_cloud_file_ids: bool,
) -> Result<(String, String), CloudStorageError> {
    let Some((last, rest)) = configured_bucket_names.split_last() else {
        return Err(CloudStorageError::EmptyBucket);
    };
    for bucket in rest {
        if let Ok(found) = decode_cloud_object_reference(
            s3_uri,
            S3_SCHEME,
            bucket,
            &BEDROCK_MANAGED_S3_PREFIXES,
            allow_legacy_cloud_file_ids,
        ) {
            return Ok(found);
        }
    }
    decode_cloud_object_reference(
        s3_uri,
        S3_SCHEME,
        last,
        &BEDROCK_MANAGED_S3_PREFIXES,
        allow_legacy_cloud_file_ids,
    )
}

// The Python BedrockError carries the rejected message as its response text and a synthetic GET request.
pub fn rejected_file_id(reason: &S3AccessError) -> Result<BedrockError, S3AccessError> {
    let message = reason.to_string();
    let input = ErrorInput {
        status_code: 400,
        message: message.clone(),
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: None,
        response: Some(ErrorReply {
            response: ErrorResponse {
                status_code: 400,
                request: ErrorRequest {
                    method: "GET".to_owned(),
                    url: REJECTED_FILE_ID_REQUEST_URL.to_owned(),
                },
            },
            headers: Vec::new(),
            body: ResponseBody::Read(message),
        }),
    };
    BedrockError::try_from(input).map_err(S3AccessError::Header)
}

fn rejection(error: &S3AccessError) -> ManagedObjectError {
    rejected_file_id(error).map_or_else(
        |source| ManagedObjectError::Configuration(Box::new(source)),
        |built| ManagedObjectError::Rejected(Box::new(built)),
    )
}

#[derive(Debug)]
pub enum ManagedObjectError {
    Configuration(Box<S3AccessError>),
    Rejected(Box<BedrockError>),
}

impl ManagedObjectError {
    #[must_use]
    pub fn status_code(&self) -> Option<u16> {
        match self {
            Self::Configuration(_) => None,
            Self::Rejected(error) => Some(error.error.status_code),
        }
    }
}

pub fn resolve_managed_s3_object(
    file_id: &str,
    params: &S3AccessParams,
    environment: &BTreeMap<String, String>,
) -> Result<(String, String), ManagedObjectError> {
    let buckets = configured_bucket_names(params, environment)
        .map_err(|error| ManagedObjectError::Configuration(Box::new(error)))?;
    let legacy = allow_legacy_cloud_file_ids(params);
    let s3_uri = decode_managed_file_id(file_id).map_err(|error| rejection(&error))?;
    validate_configured_file_id(&s3_uri, &buckets, legacy)
        .map_err(|error| rejection(&S3AccessError::Storage(error)))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingValue {
    Text(String),
    Other,
}

pub type SettingMap = BTreeMap<String, SettingValue>;

#[derive(Debug, Clone, Copy)]
pub enum S3Setting {
    EncryptionKeyId,
    BucketOwner,
}

pub struct S3SettingSources<'sources> {
    pub params: &'sources SettingMap,
    pub optional_params: Option<&'sources SettingMap>,
    pub secrets: &'sources SecretLookup,
    pub environment: &'sources BTreeMap<String, String>,
}

pub fn resolve_s3_setting(
    setting: S3Setting,
    sources: &S3SettingSources<'_>,
) -> Result<Option<SecretString>, S3AccessError> {
    let (name, variable) = match setting {
        S3Setting::EncryptionKeyId => (ENCRYPTION_KEY_ID_PARAM, ENCRYPTION_KEY_ID_ENV),
        S3Setting::BucketOwner => (BUCKET_OWNER_PARAM, BUCKET_OWNER_ENV),
    };
    for map in [Some(sources.params), sources.optional_params]
        .into_iter()
        .flatten()
    {
        if let Some(SettingValue::Text(value)) = map.get(name)
            && !value.is_empty()
        {
            return Ok(Some(SecretString::from(value.clone())));
        }
    }
    let selected = sources
        .secrets
        .get_secret_str(sources.environment, variable, None)
        .map_err(S3AccessError::Secret)?;
    Ok(selected.filter(|value| !value.expose_secret().is_empty()))
}

#[must_use]
pub fn bedrock_model_id(model: &str) -> String {
    let Some(stripped) = model.strip_prefix(BEDROCK_ROUTING_PREFIX) else {
        return model.to_owned();
    };
    stripped.to_owned()
}

#[must_use]
pub fn strip_llm_routing_prefix(model: &str, context: &ResolutionContext<'_>) -> String {
    let request = ResolutionRequest {
        model,
        custom_llm_provider: None,
        api_base: None,
        api_key: None,
    };
    match resolve(&request, context) {
        Ok(resolved) => resolved.model,
        Err(source) => {
            tracing::error!(
                error = %source,
                "litellm.llms.bedrock.files.transformation.py::_strip_llm_routing_prefix() - Error inferring custom_llm_provider"
            );
            model.to_owned()
        }
    }
}

#[must_use]
pub fn invoke_route_model(model: &str, context: &ResolutionContext<'_>) -> String {
    let stripped = strip_llm_routing_prefix(model, context);
    let rest = stripped
        .strip_prefix(INVOKE_ROUTE_PREFIX)
        .unwrap_or(&stripped);
    format!("{INVOKE_ROUTE_PREFIX}{rest}")
}

pub fn parse_s3_uri(s3_uri: &str) -> Result<(String, String), S3AccessError> {
    let Some(path) = s3_uri.strip_prefix(S3_SCHEME) else {
        return Err(S3AccessError::InvalidS3Uri {
            uri: s3_uri.to_owned(),
        });
    };
    let Some((bucket, key)) = path.split_once('/') else {
        return Err(S3AccessError::InvalidS3Uri {
            uri: s3_uri.to_owned(),
        });
    };
    Ok((bucket.to_owned(), key.to_owned()))
}

// The B590 base state every Bedrock batch or files object carries: the shared IAM credential cache.
pub struct AwsBase {
    pub credential_cache: Arc<CredentialCache>,
}

pub struct BatchFiles {
    pub base: AwsBase,
}

pub struct BedrockFilesHandler {
    pub base: AwsBase,
    pub client: ProxiedClient,
}

mod tests;
