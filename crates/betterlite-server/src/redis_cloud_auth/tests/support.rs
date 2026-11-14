use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use aws_credential_types::Credentials;
use aws_credential_types::provider::SharedCredentialsProvider;
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse as _, Response};
use axum::routing::post;
use betterlite_azure::azure_arc_identity::{ArcTransport, MetadataRequest, MetadataResponse};
use betterlite_azure::azure_client_secret::transport::{
    HttpReply as ClientSecretReply, HttpRequest as ClientSecretRequest,
    Transport as ClientSecretTransport,
};
use betterlite_azure::azure_hosted_identity::request::{
    HostedRequest, HostedResponse, HostedTransport,
};
use betterlite_azure::azure_managed_identity::imds::ImdsTransport;
use betterlite_azure::azure_username_password::transport::{
    HttpRequest as PasswordRequest, Transport as PasswordTransport,
};
use betterlite_azure::oidc_exchange::transport::{
    HttpReply as OidcReply, Transport as OidcTransport,
};
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::redis_credentials::aws::AwsSettings;
use betterlite_config::sections::redis_credentials::azure::{AzureCredential, AzureSettings};
use betterlite_config::sections::redis_credentials::gcp::{GcpSettings, GcpTokenCache};
use betterlite_config::sections::redis_credentials::{Provider, Settings};
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_vertex::credential_sources::error::CredentialSourceError;
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::runtime::Handle;
use tokio::task::JoinSet;

use super::super::CloudRedisAuthDeps;
use super::super::azure::{AzureGrantError, AzureGrants};
use super::super::gcp::{AdcError, AdcTokenSource};

pub(super) fn proxied_client() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    ProxiedClient::outbound(&Arc::new(policy)).unwrap()
}

pub(super) async fn run_blocking<T, F>(work: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let mut tasks = JoinSet::new();
    tasks.spawn_blocking(work);
    tasks.join_next().await.unwrap().unwrap()
}

#[derive(Clone, Debug)]
pub(super) struct Seen {
    pub method: String,
    pub path: String,
    pub authorization: Option<String>,
    pub content_type: Option<String>,
    pub body: String,
}

#[derive(Clone)]
pub(super) struct IamState {
    pub seen: Arc<Mutex<Vec<Seen>>>,
    pub status: u16,
    pub body: String,
}

pub(super) struct IamServer {
    pub base: String,
    pub seen: Arc<Mutex<Vec<Seen>>>,
    pub _tasks: JoinSet<()>,
}

async fn record_iam(
    State(state): State<IamState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> Response {
    let seen = Seen {
        method: method.as_str().to_owned(),
        path: uri.path().to_owned(),
        authorization: headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
        content_type: headers
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
        body,
    };
    state
        .seen
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(seen);
    let status = StatusCode::from_u16(state.status).unwrap();
    (status, state.body).into_response()
}

pub(super) async fn iam_server(status: u16, body: &str) -> IamServer {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let state = IamState {
        seen: Arc::clone(&seen),
        status,
        body: body.to_owned(),
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let application = Router::new()
        .route("/v1/{*path}", post(record_iam))
        .with_state(state);
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        axum::serve(listener, application).await.unwrap();
    });
    IamServer {
        base,
        seen,
        _tasks: tasks,
    }
}

pub(super) struct FixedAdc {
    pub token: Option<&'static str>,
}

impl AdcTokenSource for FixedAdc {
    fn access_token(&self, _scope: &str) -> Result<SecretString, AdcError> {
        self.token.map_or_else(
            || {
                Err(AdcError::Adc {
                    source: CredentialSourceError::MissingProject,
                })
            },
            |token| Ok(SecretString::from(token)),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum GrantCall {
    ClientSecret {
        tenant_id: String,
        client_id: String,
        scope: String,
    },
    ManagedIdentity {
        client_id: String,
        scope: String,
    },
    Default {
        scope: String,
    },
}

pub(super) struct RecordingGrants {
    // None makes every grant refuse, so a test can assert how a failure surfaces
    pub token: Option<&'static str>,
    pub calls: Mutex<Vec<GrantCall>>,
}

impl RecordingGrants {
    pub(super) const fn new(token: &'static str) -> Self {
        Self {
            token: Some(token),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) const fn refusing() -> Self {
        Self {
            token: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<GrantCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn grant(&self, call: GrantCall) -> Result<SecretString, AzureGrantError> {
        let kind = match call {
            GrantCall::ClientSecret { .. } => "client secret",
            GrantCall::ManagedIdentity { .. } => "managed identity",
            GrantCall::Default { .. } => "default",
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        self.token.map_or_else(
            || {
                Err(AzureGrantError::Grant {
                    kind,
                    message: "the test grant refuses".to_owned(),
                })
            },
            |token| Ok(SecretString::from(token)),
        )
    }
}

impl AzureGrants for RecordingGrants {
    fn client_secret(
        &self,
        tenant_id: &str,
        client_id: &str,
        _client_secret: &SecretString,
        scope: &str,
    ) -> Result<SecretString, AzureGrantError> {
        self.grant(GrantCall::ClientSecret {
            tenant_id: tenant_id.to_owned(),
            client_id: client_id.to_owned(),
            scope: scope.to_owned(),
        })
    }

    fn managed_identity(
        &self,
        client_id: &str,
        scope: &str,
    ) -> Result<SecretString, AzureGrantError> {
        self.grant(GrantCall::ManagedIdentity {
            client_id: client_id.to_owned(),
            scope: scope.to_owned(),
        })
    }

    fn default_credential(&self, scope: &str) -> Result<SecretString, AzureGrantError> {
        let token = self.token.ok_or_else(|| AzureGrantError::Grant {
            kind: "default",
            message: "the test grant refuses".to_owned(),
        })?;
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(GrantCall::Default {
                scope: scope.to_owned(),
            });
        Ok(SecretString::from(token))
    }
}

pub(super) fn deps(
    runtime: Handle,
    adc: Arc<dyn AdcTokenSource>,
    azure: Arc<dyn AzureGrants>,
    aws: SharedCredentialsProvider,
) -> CloudRedisAuthDeps {
    CloudRedisAuthDeps {
        runtime,
        http: proxied_client(),
        adc,
        azure,
        aws,
    }
}

pub(super) fn sourced_settings(provider: Provider) -> Settings {
    Settings {
        provider: Some(provider),
        url: None,
        username: None,
        password: None,
        warnings: Vec::new(),
        environment_snapshot: BTreeMap::new(),
    }
}

pub(super) fn gcp_provider() -> Provider {
    Provider::Gcp(GcpSettings {
        service_account: SecretString::from(
            "projects/-/serviceAccounts/sa@project.iam.gserviceaccount.com".to_owned(),
        ),
        ssl_ca_certs: None,
        token_ttl_seconds: 3300,
        token_cache: Arc::new(GcpTokenCache::default()),
    })
}

pub(super) fn azure_settings(credential: AzureCredential) -> AzureSettings {
    AzureSettings {
        client_id: Some(SecretString::from("client".to_owned())),
        tenant_id: Some(SecretString::from("tenant".to_owned())),
        client_secret: Some(SecretString::from("secret".to_owned())),
        username: Some("redis-user".to_owned()),
        credential,
    }
}

pub(super) fn azure_provider(credential: AzureCredential) -> Provider {
    Provider::Azure(azure_settings(credential))
}

pub(super) fn aws_settings() -> AwsSettings {
    AwsSettings {
        user_name: "app".to_owned(),
        cache_name: "cache.example".to_owned(),
        region: "eu-west-1".to_owned(),
        serverless: false,
        token_ttl_seconds: 900,
    }
}

pub(super) fn aws_provider() -> Provider {
    Provider::Aws(aws_settings())
}

pub(super) fn static_aws_provider() -> SharedCredentialsProvider {
    SharedCredentialsProvider::new(Credentials::new(
        "AKIAFIXTURE",
        "fixture-secret",
        None,
        None,
        "betterlite-test",
    ))
}

pub(super) struct Reply {
    pub url_contains: &'static str,
    pub status: u16,
    pub body: &'static str,
}

pub(super) struct FakeIo {
    pub replies: Mutex<VecDeque<Reply>>,
    // each IMDS token request takes the next body, and an empty queue makes the endpoint unreachable
    pub imds_bodies: Mutex<VecDeque<&'static str>>,
    pub requests: Mutex<Vec<String>>,
}

impl FakeIo {
    pub(super) fn new(replies: Vec<Reply>, imds_bodies: Vec<&'static str>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            imds_bodies: Mutex::new(VecDeque::from(imds_bodies)),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ClientSecretTransport for FakeIo {
    fn send(&self, request: &ClientSecretRequest) -> io::Result<ClientSecretReply> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.url.clone());
        let mut replies = self.replies.lock().unwrap_or_else(PoisonError::into_inner);
        let index = replies
            .iter()
            .position(|reply| request.url.contains(reply.url_contains))
            .ok_or_else(|| io::Error::other(format!("no reply for {}", request.url)))?;
        let reply = replies.remove(index).unwrap();
        drop(replies);
        Ok(ClientSecretReply {
            status: reply.status,
            body: SecretString::from(reply.body),
        })
    }
}

impl PasswordTransport for FakeIo {
    fn send(&self, _request: &PasswordRequest) -> io::Result<ClientSecretReply> {
        Err(io::Error::other("the fixture has no password endpoint"))
    }
}

impl OidcTransport for FakeIo {
    fn post_form(&self, _url: &str, _form: &[(String, SecretString)]) -> io::Result<OidcReply> {
        Err(io::Error::other("the fixture has no OIDC endpoint"))
    }
}

impl ImdsTransport for FakeIo {
    fn get(&self, request: &MetadataRequest) -> io::Result<MetadataResponse> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.url.clone());
        let body = self
            .imds_bodies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .ok_or_else(|| io::Error::other("the fixture has no IMDS endpoint"))?;
        Ok(MetadataResponse {
            status: 200,
            headers: Vec::new(),
            body: SecretString::from(body),
        })
    }
}

impl HostedTransport for FakeIo {
    fn send(&self, _request: &HostedRequest) -> io::Result<HostedResponse> {
        Err(io::Error::other("the fixture has no hosted endpoint"))
    }
}

impl ArcTransport for FakeIo {
    fn get(&self, _request: &MetadataRequest) -> io::Result<MetadataResponse> {
        Err(io::Error::other("the fixture has no Arc endpoint"))
    }
}
