use std::collections::BTreeMap;
use std::future::ready;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aws_credential_types::Credentials;
use http::{HeaderMap, StatusCode};
use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;

use crate::aws::request_signing::{
    ActionRequest, AwsCredentialsInfo, AwsOptionalParams, AwsTransport, CredentialResolver,
    TransportError, TransportFuture, TransportRequest, TransportResponse,
};
use crate::aws::{AwsDependencies, AwsSecretsManager};

pub(super) const ACCESS_KEY: &str = "AKIDEXAMPLE";
pub(super) const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";
pub(super) const SESSION_TOKEN: &str = "session-token";

pub(super) fn signing_time() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(1_709_641_845))
        .unwrap()
}

pub(super) fn dns_suffix(region: &str) -> &'static str {
    if region.starts_with("cn-") {
        "amazonaws.com.cn"
    } else {
        "amazonaws.com"
    }
}

#[derive(Debug, Error)]
pub(super) enum ResolverFailure {
    #[error("credentials are unavailable")]
    Unavailable,
}

const TEXT_KEYS: [&str; 7] = [
    "role",
    "session_name",
    "region",
    "external_id",
    "profile",
    "web_identity_token",
    "sts_endpoint",
];

type Reader = fn(&AwsOptionalParams) -> Option<&str>;

const READERS: [(&str, Reader); 8] = [
    ("role", |params| params.aws_role_name.as_deref()),
    ("session_name", |params| params.aws_session_name.as_deref()),
    ("region", |params| params.aws_region_name.as_deref()),
    ("external_id", |params| params.aws_external_id.as_deref()),
    ("profile", |params| params.aws_profile_name.as_deref()),
    ("web_identity_token", |params| {
        params
            .aws_web_identity_token
            .as_ref()
            .map(ExposeSecret::expose_secret)
    }),
    ("sts_endpoint", |params| params.aws_sts_endpoint.as_deref()),
    ("access_key_id", |params| {
        params
            .aws_access_key_id
            .as_ref()
            .map(ExposeSecret::expose_secret)
    }),
];

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct Snapshot(pub(super) BTreeMap<&'static str, String>);

impl Snapshot {
    pub(super) fn filled(prefix: &str) -> Self {
        Self(
            TEXT_KEYS
                .into_iter()
                .map(|key| (key, format!("{prefix}-{key}")))
                .collect(),
        )
    }

    pub(super) fn empty_strings() -> Self {
        Self(
            TEXT_KEYS
                .into_iter()
                .map(|key| (key, String::new()))
                .collect(),
        )
    }

    pub(super) fn into_params(self) -> AwsOptionalParams {
        let mut params = AwsOptionalParams::default();
        for (key, text) in self.0 {
            match key {
                "role" => params.aws_role_name = Some(text),
                "session_name" => params.aws_session_name = Some(text),
                "region" => params.aws_region_name = Some(text),
                "external_id" => params.aws_external_id = Some(text),
                "profile" => params.aws_profile_name = Some(text),
                "web_identity_token" => {
                    params.aws_web_identity_token = Some(SecretString::from(text));
                }
                "sts_endpoint" => params.aws_sts_endpoint = Some(text),
                _ => params.aws_access_key_id = Some(SecretString::from(text)),
            }
        }
        params
    }

    pub(super) fn of(params: &AwsOptionalParams) -> Self {
        Self(
            READERS
                .iter()
                .filter_map(|(key, read)| read(params).map(|text| (*key, text.to_owned())))
                .collect(),
        )
    }
}

#[derive(Debug)]
pub(super) struct FakeResolver {
    pub(super) region: String,
    pub(super) endpoint: Option<String>,
    pub(super) has_session_token: bool,
    pub(super) fail: bool,
    pub(super) seen: Mutex<Vec<Snapshot>>,
}

impl CredentialResolver for FakeResolver {
    type Error = ResolverFailure;

    fn resolve_credentials(
        &self,
        optional_params: &mut AwsOptionalParams,
    ) -> Result<AwsCredentialsInfo, ResolverFailure> {
        self.seen
            .lock()
            .map_err(|_poisoned| ResolverFailure::Unavailable)?
            .push(Snapshot::of(optional_params));
        optional_params.aws_role_name = None;
        if self.fail {
            return Err(ResolverFailure::Unavailable);
        }
        Ok(AwsCredentialsInfo {
            aws_region_name: self.region.clone(),
            aws_bedrock_runtime_endpoint: self.endpoint.clone(),
            credentials: Credentials::new(
                ACCESS_KEY,
                SECRET_KEY,
                self.has_session_token.then(|| SESSION_TOKEN.to_owned()),
                None,
                "test",
            ),
        })
    }
}

#[derive(Debug)]
pub(super) struct Sent {
    pub(super) url: String,
    pub(super) headers: HeaderMap,
    pub(super) body: Vec<u8>,
    pub(super) timeout: Duration,
    pub(super) max_response_bytes: usize,
}

#[derive(Debug)]
pub(super) struct FakeTransport {
    reply: Mutex<Option<Result<TransportResponse, TransportError>>>,
    pub(super) sent: Mutex<Vec<Sent>>,
}

impl AwsTransport for FakeTransport {
    fn post<'request>(
        &'request self,
        request: TransportRequest<'request>,
    ) -> TransportFuture<'request> {
        self.sent.lock().unwrap().push(Sent {
            url: request.url.to_owned(),
            headers: request.headers,
            body: request.body.to_vec(),
            timeout: request.timeout,
            max_response_bytes: request.max_response_bytes,
        });
        let reply = self.reply.lock().unwrap().take().unwrap();
        Box::pin(ready(reply))
    }
}

pub(super) fn transport(reply: Result<TransportResponse, TransportError>) -> Arc<FakeTransport> {
    Arc::new(FakeTransport {
        reply: Mutex::new(Some(reply)),
        sent: Mutex::new(Vec::new()),
    })
}

pub(super) fn answer(status: u16, body: &[u8]) -> Arc<FakeTransport> {
    transport(Ok(TransportResponse {
        status: StatusCode::from_u16(status).unwrap(),
        body: body.to_vec(),
    }))
}

pub(super) fn resolver(region: &str) -> FakeResolver {
    FakeResolver {
        region: region.to_owned(),
        endpoint: None,
        has_session_token: true,
        fail: false,
        seen: Mutex::new(Vec::new()),
    }
}

pub(super) fn manager(
    resolver: FakeResolver,
    environment: BTreeMap<String, String>,
) -> AwsSecretsManager<FakeResolver> {
    AwsSecretsManager {
        aws_region_name: None,
        aws_role_name: None,
        aws_session_name: None,
        aws_external_id: None,
        aws_profile_name: None,
        aws_web_identity_token: None,
        aws_sts_endpoint: None,
        replica_regions: Vec::new(),
        kms_key_id: None,
        dependencies: AwsDependencies {
            environment,
            transport: answer(200, b"{}"),
            resolver,
            dns_suffix,
        },
    }
}

pub(super) fn manager_with_instance_settings(
    resolver: FakeResolver,
    settings: &Snapshot,
) -> AwsSecretsManager<FakeResolver> {
    let mut client = manager(resolver, BTreeMap::new());
    client.aws_role_name = settings.0.get("role").cloned();
    client.aws_session_name = settings.0.get("session_name").cloned();
    client.aws_region_name = settings.0.get("region").cloned();
    client.aws_external_id = settings.0.get("external_id").cloned();
    client.aws_profile_name = settings.0.get("profile").cloned();
    client.aws_web_identity_token = settings
        .0
        .get("web_identity_token")
        .cloned()
        .map(SecretString::from);
    client.aws_sts_endpoint = settings.0.get("sts_endpoint").cloned();
    client
}

pub(super) fn get_secret_request(name: &str) -> ActionRequest<'_> {
    ActionRequest {
        action: "GetSecretValue",
        secret_name: name,
        request_data: None,
    }
}
