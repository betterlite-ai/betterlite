use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::future::ready;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::record::ModelInfo;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::files::records::CreatePurpose;
use betterlite_prompts::media_metadata::{FileBody, FileData};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use indexmap::IndexMap;
use secrecy::SecretString;
use tokio::runtime::{Builder, Runtime};
use uuid::Uuid;

use crate::aws_settings::sources::Credentials;
use crate::batch_records::error::BatchRecordsError;
use crate::batch_records::families::{BatchFamilies, Failure, Params};
use crate::batch_records::jsonl::{BatchRecordContext, transform_openai_jsonl};
use crate::batch_records::record::{
    BedrockBatchRecord, OpenAiBatchRecord, decode_batch_input_line,
};
use crate::batch_records::upload::{
    CreateFileRequest, FileUploadHandoff, transform_create_file_request,
};
use crate::bedrock_model_capabilities::ModelInfoLookup;
use crate::credentials::error::CredentialError;
use crate::request_policy::{Params as RequestParams, RequestValue};
use crate::signing::target::RegionInputs;

type Attempt<T> = Result<T, Failure>;

pub(super) const FIXED_UUID: Uuid = Uuid::from_u128(0x0000_0000_0000_4000_8000_0000_0000_0001);
pub(super) const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";
pub(super) const OTHER_SECRET_KEY: &str = "other-secret-key-for-isolation-tests";
pub(super) const ACCESS_KEY: &str = "AKIDEXAMPLE";
pub(super) const OTHER_ACCESS_KEY: &str = "AKIDOTHEREXAMPLE";
pub(super) const GATEWAY_MODEL: &str = "openai.gpt-oss-20b-1:0";

fn runtime() -> Runtime {
    Builder::new_current_thread()
        .build()
        .expect("the test runtime builds")
}

pub(super) fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("the fixture JSON parses")
}

pub(super) fn value(json: &Json) -> serde_json::Value {
    serde_json::to_value(json).expect("the fixture value renders")
}

pub(super) fn params(source: &str) -> Params {
    serde_json::from_str(source).expect("the fixture params parse")
}

fn text_params(entries: &[(&str, &str)]) -> RequestParams {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), RequestValue::Text((*value).to_owned())))
        .collect()
}

#[derive(Debug, Clone)]
pub(super) struct Call {
    pub(super) model: String,
    pub(super) messages: Vec<Json>,
    pub(super) optional_params: Params,
}

impl Call {
    fn of(model: &str, messages: &[Json], params: &Params) -> Self {
        Self {
            model: model.to_owned(),
            messages: messages.to_vec(),
            optional_params: params.clone(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fixture holds no body")]
struct MissingFixture;

#[derive(Default)]
pub(super) struct Families {
    pub(super) nova_calls: Mutex<Vec<Call>>,
    pub(super) anthropic_params_calls: Mutex<Vec<(String, Params)>>,
    pub(super) anthropic_body_calls: Mutex<Vec<Call>>,
    pub(super) responses_calls: Mutex<Vec<Json>>,
    pub(super) nova_body: Option<Json>,
    pub(super) anthropic_params: Option<Params>,
    pub(super) anthropic_body: Option<Json>,
    pub(super) responses_body: Option<Json>,
    pub(super) batch: bool,
}

impl Families {
    pub(super) fn batch() -> Self {
        let mut families = Self::default();
        families.batch = true;
        families.nova_body = Some(json(r#"{"messages":[],"inferenceConfig":{}}"#));
        families
    }

    pub(super) fn nova_calls(&self) -> Vec<Call> {
        self.nova_calls
            .lock()
            .expect("no panic held the lock")
            .clone()
    }

    pub(super) fn anthropic_params_calls(&self) -> Vec<(String, Params)> {
        self.anthropic_params_calls
            .lock()
            .expect("no panic held the lock")
            .clone()
    }

    pub(super) fn anthropic_body_calls(&self) -> Vec<Call> {
        self.anthropic_body_calls
            .lock()
            .expect("no panic held the lock")
            .clone()
    }

    pub(super) fn responses_calls(&self) -> Vec<Json> {
        self.responses_calls
            .lock()
            .expect("no panic held the lock")
            .clone()
    }
}

#[async_trait::async_trait]
impl BatchFamilies for Families {
    async fn anthropic_params(&self, mapping_model: &str, params: &Params) -> Attempt<Params> {
        self.anthropic_params_calls
            .lock()
            .expect("no panic held the lock")
            .push((mapping_model.to_owned(), params.clone()));
        match &self.anthropic_params {
            Some(mapped) => Ok(mapped.clone()),
            None => Ok(Params::new()),
        }
    }

    async fn anthropic_body(
        &self,
        model: &str,
        messages: &[Json],
        params: &Params,
    ) -> Attempt<Json> {
        let call = Call::of(model, messages, params);
        self.anthropic_body_calls
            .lock()
            .expect("no panic held the lock")
            .push(call);
        self.anthropic_body
            .clone()
            .ok_or_else(|| Failure::from(MissingFixture))
    }

    async fn nova_body(&self, model: &str, messages: &[Json], params: &Params) -> Attempt<Json> {
        self.nova_calls
            .lock()
            .expect("no panic held the lock")
            .push(Call::of(model, messages, params));
        Ok(self.nova_body.clone().unwrap_or(Json::Null))
    }

    async fn responses_to_chat(&self, body: &Json) -> Attempt<Json> {
        self.responses_calls
            .lock()
            .expect("no panic held the lock")
            .push(body.clone());
        let recorded = self.responses_body.clone();
        Ok(recorded.unwrap_or(Json::Null))
    }

    fn is_batch_jsonl(
        &self,
        _purpose: Option<&str>,
        _content_type: Option<&str>,
        _body: Option<&[u8]>,
    ) -> bool {
        self.batch
    }
}

struct StubLookup;

impl ModelInfoLookup for StubLookup {
    type Error = Infallible;

    fn bedrock_model_info(
        &self,
        _model: &str,
    ) -> impl std::future::Future<Output = Result<ModelInfo, Infallible>> + Send {
        ready(Ok(ModelInfo {
            fields: IndexMap::new(),
        }))
    }
}

struct Sources {
    catalog: ModelCatalog,
    known: BTreeSet<String>,
    custom: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl Sources {
    fn load() -> Self {
        Self {
            catalog: ModelCatalog::parse(b"{}", UnknownFields::Warn)
                .expect("an empty catalog parses"),
            known: BTreeSet::new(),
            custom: BTreeSet::new(),
            environment: BTreeMap::new(),
        }
    }

    fn resolution(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            registry: JSONProviderRegistry::load(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
            catalog: &self.catalog,
            known_model_names: &self.known,
            custom_providers: &self.custom,
            environment: &self.environment,
        }
    }
}

struct NoOidc;

impl OidcHttp for NoOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: String::from("the fixture has no OIDC transport"),
        })
    }
}

impl AzureOidcTokens for NoOidc {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

impl OidcClock for NoOidc {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

fn secret_lookup() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: std::sync::Arc::new(NoOidc),
        azure_tokens: std::sync::Arc::new(NoOidc),
        clock: std::sync::Arc::new(NoOidc),
        oidc_cache: OidcTokenCache::default(),
    }
}

pub(super) fn record(source: &str) -> OpenAiBatchRecord {
    decode_batch_input_line(source.as_bytes())
        .expect("the record line decodes")
        .expect("the record line is not blank")
}

pub(super) fn transform(
    families: &Families,
    source: &str,
    target_model: &str,
) -> Result<Vec<Json>, BatchRecordsError> {
    let record = record(source);
    let aliases = BTreeMap::new();
    transform_with(families, &[record], target_model, &aliases)
}

pub(super) fn transform_with(
    families: &Families,
    records: &[OpenAiBatchRecord],
    target_model: &str,
    aliases: &BTreeMap<String, String>,
) -> Result<Vec<Json>, BatchRecordsError> {
    let sources = Sources::load();
    let resolution = sources.resolution();
    let lookup = StubLookup;
    let context = BatchRecordContext {
        aliases,
        families,
        lookup: &lookup,
        resolution: &resolution,
    };
    runtime()
        .block_on(transform_openai_jsonl(records, target_model, &context))
        .map(|records| records.iter().map(BedrockBatchRecord::to_json).collect())
}

pub(super) fn error_of(result: Result<Vec<Json>, BatchRecordsError>) -> BatchRecordsError {
    result.expect_err("the fixture input is refused")
}

pub(super) fn batch_file() -> FileData {
    FileData::Tuple3 {
        filename: None,
        content: FileBody::Bytes(
            b"{\"custom_id\":\"a\",\"body\":{\"model\":\"openai.gpt-oss-20b-1:0\",\"messages\":[{\"content\":\"hi\",\"role\":\"user\"}]}}"
                .to_vec(),
        ),
        content_type: Some(String::from("application/jsonl")),
    }
}

pub(super) fn booking_params() -> RequestParams {
    let mut params = text_params(&[
        ("s3_bucket_name", "my-bucket"),
        ("s3_region_name", "us-east-1"),
        ("s3_access_key_id", ACCESS_KEY),
    ]);
    params.insert(
        String::from("s3_secret_access_key"),
        RequestValue::Secret(SecretString::from(SECRET_KEY)),
    );
    params
}

fn resolve_credentials(
    _settings: &AwsSettings,
    _region: &str,
) -> std::future::Ready<Result<Credentials, CredentialError>> {
    ready(Ok(Credentials {
        access_key_id: SecretString::from(ACCESS_KEY),
        secret_access_key: SecretString::from(SECRET_KEY),
        session_token: None,
    }))
}

pub(super) fn run_upload(
    families: &Families,
    file: &FileData,
    purpose: Option<CreatePurpose>,
    model: &str,
    litellm_params: &mut RequestParams,
) -> (Result<FileUploadHandoff, BatchRecordsError>, RequestParams) {
    let result = upload_request(families, Some(file), purpose, model, litellm_params);
    (result, litellm_params.clone())
}

pub(super) fn run_upload_missing_file(families: &Families) -> (BatchRecordsError, RequestParams) {
    let mut litellm_params = booking_params();
    let result = upload_request(families, None, None, "", &mut litellm_params);
    let error = result.expect_err("the upload without a file is refused");
    (error, litellm_params)
}

fn upload_request(
    families: &Families,
    file: Option<&FileData>,
    purpose: Option<CreatePurpose>,
    model: &str,
    litellm_params: &mut RequestParams,
) -> Result<FileUploadHandoff, BatchRecordsError> {
    let sources = Sources::load();
    let aliases = BTreeMap::new();
    let lookup = StubLookup;
    let secrets = secret_lookup();
    let optional_params = RequestParams::new();
    let resolution = sources.resolution();
    let context = BatchRecordContext {
        aliases: &aliases,
        families,
        lookup: &lookup,
        resolution: &resolution,
    };
    let mut request = CreateFileRequest {
        model,
        file,
        purpose,
        optional_params: &optional_params,
        litellm_params,
        environment: &sources.environment,
        secrets: &secrets,
        region_inputs: RegionInputs {
            environment_region_name: None,
            environment_region: None,
            session_region: None,
        },
        id: FIXED_UUID,
        now: SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
    };
    runtime().block_on(transform_create_file_request(
        &mut request,
        &context,
        resolve_credentials,
    ))
}
