use std::collections::BTreeMap;
use std::time::SystemTime;

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_models::decisions::Json;
use betterlite_prompts::gitlab_client::payload::decode_text;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use secrecy::ExposeSecret as _;
use serde_json::value::RawValue;

use super::decode::decode_async_embedding_status;
use super::error::{EmbeddingJobsError, FailureBody};
use super::logging::{StatusEvent, StatusLogger};
use super::transport::{StatusReply, StatusTransport, StatusWireRequest};
use super::url::{StatusEndpoint, async_invoke_status_url};
use crate::aws_settings::sources::Credentials;
use crate::bedrock_embedding_io::credentials::{
    CredentialRequest, ManagedRegionWorker, load_credentials,
};
use crate::bedrock_embedding_io::signed_get::sign_get_request;
use crate::bedrock_embedding_io::transport::{ASYNC_CLIENT_PROVIDER, ClientFactory};
use crate::credentials::CredentialSource;
use crate::signing::executor::AwsSigningExecutor;

const RUNTIME_ENDPOINT_VARIABLE: &str = "AWS_BEDROCK_RUNTIME_ENDPOINT";
const ENDPOINT_PARAMETER: &str = "aws_bedrock_runtime_endpoint";

pub struct AsyncInvokeStatusCall<'source> {
    pub invocation_arn: &'source str,
    pub region: &'source str,
    pub optional_params: &'source mut BTreeMap<String, Box<RawValue>>,
    pub environment: &'source BTreeMap<String, String>,
    pub now: SystemTime,
}

pub struct StatusServices<'source, Source, Factory> {
    pub source: &'source Source,
    pub secrets: &'source SecretLookup,
    pub region_worker: &'source ManagedRegionWorker,
    pub factory: &'source Factory,
    pub signing: &'source AwsSigningExecutor,
}

struct PreparedStatusRequest {
    url: String,
    headers: Vec<(String, String)>,
    credentials: Credentials,
}

pub async fn get_async_invoke_status<Source, Factory>(
    mut call: AsyncInvokeStatusCall<'_>,
    services: &StatusServices<'_, Source, Factory>,
    mut log: Option<&mut StatusLogger<'_>>,
) -> Result<Json, EmbeddingJobsError>
where
    Source: CredentialSource,
    Factory: ClientFactory + Sync,
    Factory::Client: StatusTransport,
{
    let prepared = prepare_status_request(&mut call, services).await?;
    if let Some(context) = log.as_deref_mut() {
        context(
            call.invocation_arn,
            StatusEvent::Request {
                url: &prepared.url,
                headers: &prepared.headers,
            },
        )?;
    }
    let reply = send_status_request(&prepared, services.factory).await?;
    complete_status_request(call.invocation_arn, &reply, &prepared.credentials, log)
}

async fn prepare_status_request<Source, Factory>(
    call: &mut AsyncInvokeStatusCall<'_>,
    services: &StatusServices<'_, Source, Factory>,
) -> Result<PreparedStatusRequest, EmbeddingJobsError>
where
    Source: CredentialSource,
    Factory: Sync,
{
    let configured = configured_endpoint(call.optional_params)?;
    let endpoint = StatusEndpoint {
        configured_endpoint: configured.as_deref(),
        environment_endpoint: call
            .environment
            .get(RUNTIME_ENDPOINT_VARIABLE)
            .map(String::as_str),
        region: call.region,
    };
    let url = async_invoke_status_url(endpoint, call.invocation_arn);
    let credentials = status_credentials(call, services).await?;
    let headers = vec![("Content-Type".to_owned(), "application/json".to_owned())];
    // the explicit method region signs the request even when credential loading resolved another
    let signer_credentials = credentials.clone();
    let region = call.region.to_owned();
    let now = call.now;
    let signed = services
        .signing
        .run(move || sign_get_request(&signer_credentials, &url, &headers, &region, now))
        .await??;
    Ok(PreparedStatusRequest {
        url: signed.url,
        headers: prepared_headers(&signed.headers),
        credentials,
    })
}

async fn status_credentials<Source, Factory>(
    call: &mut AsyncInvokeStatusCall<'_>,
    services: &StatusServices<'_, Source, Factory>,
) -> Result<Credentials, EmbeddingJobsError>
where
    Source: CredentialSource,
    Factory: Sync,
{
    let loaded = load_credentials(
        CredentialRequest {
            optional_params: call.optional_params,
            environment: call.environment,
            bearer_token: None,
        },
        services.secrets,
        services.region_worker,
        |settings: &AwsSettings, region: &str| {
            services.source.resolve(settings.clone(), region.to_owned())
        },
    )
    .await?;
    loaded
        .credentials
        .ok_or(EmbeddingJobsError::MissingCredentials)
}

fn configured_endpoint(
    parameters: &BTreeMap<String, Box<RawValue>>,
) -> Result<Option<String>, EmbeddingJobsError> {
    let Some(raw) = parameters.get(ENDPOINT_PARAMETER) else {
        return Ok(None);
    };
    serde_json::from_str::<Option<String>>(raw.get())
        .map_err(|source| EmbeddingJobsError::Endpoint { source })
}

// botocore appends the token before the authorization header; aws-sigv4 returns it after
fn prepared_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    let mut ordered: Vec<(String, String)> = headers
        .iter()
        .filter(|(name, _)| !name.eq_ignore_ascii_case("authorization"))
        .cloned()
        .collect();
    ordered.extend(
        headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
            .cloned(),
    );
    ordered
}

async fn send_status_request<Factory>(
    prepared: &PreparedStatusRequest,
    factory: &Factory,
) -> Result<StatusReply, EmbeddingJobsError>
where
    Factory: ClientFactory + Sync,
    Factory::Client: StatusTransport,
{
    // the B173 client is fetched with no per-call timeout, so it keeps the shared one
    factory
        .async_client(ASYNC_CLIENT_PROVIDER, None)
        .get(StatusWireRequest {
            url: prepared.url.clone(),
            headers: prepared.headers.clone(),
        })
        .await
        .map_err(EmbeddingJobsError::from)
}

fn complete_status_request(
    invocation_arn: &str,
    reply: &StatusReply,
    credentials: &Credentials,
    log: Option<&mut StatusLogger<'_>>,
) -> Result<Json, EmbeddingJobsError> {
    let text = scrub_credentials(decode_text(&reply.body), credentials);
    if let Some(context) = log {
        context(invocation_arn, StatusEvent::Reply(&text))?;
    }
    // only an exact 200 is a status body; a Failed job in a 200 body is still a success
    if reply.status == 200 {
        return decode_async_embedding_status(&reply.body);
    }
    Err(EmbeddingJobsError::HttpFailure {
        status: reply.status,
        body: FailureBody(text),
    })
}

// An upstream error or callback must not echo the credentials used for this request.
fn scrub_credentials(mut text: String, credentials: &Credentials) -> String {
    let secrets = [
        Some(&credentials.access_key_id),
        Some(&credentials.secret_access_key),
        credentials.session_token.as_ref(),
    ];
    for secret in secrets.into_iter().flatten() {
        if !secret.expose_secret().is_empty() {
            text = text.replace(secret.expose_secret(), "[REDACTED]");
        }
    }
    text
}
