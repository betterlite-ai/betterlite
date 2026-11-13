use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::redis_credentials::aws::AwsSettings;
use betterlite_config::sections::redis_credentials::azure::{AzureCredential, AzureSettings};
use betterlite_config::sections::redis_credentials::gcp::{GcpSettings, GcpTokenCache};
use betterlite_config::sections::redis_credentials::{Provider, Settings};
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::runtime::Handle;
use tokio::task::JoinSet;

use super::super::CloudRedisAuthDeps;
use super::super::azure::{AzureGrantError, AzureGrants};
use super::super::gcp::{AdcError, AdcTokenSource};
use betterlite_vertex::credential_sources::error::CredentialSourceError;

pub fn proxied_client() -> ProxiedClient {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    ProxiedClient::outbound(&Arc::new(policy)).unwrap()
}

#[derive(Clone, Debug)]
pub struct Seen {
    pub method: String,
    pub path: String,
    pub authorization: Option<String>,
    pub content_type: Option<String>,
    pub body: String,
}

#[derive(Clone)]
pub struct IamState {
    pub seen: Arc<Mutex<Vec<Seen>>>,
    pub status: u16,
    pub body: String,
}

pub struct IamServer {
    pub base: String,
    pub seen: Arc<Mutex<Vec<Seen>>>,
    pub tasks: JoinSet<()>,
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
    (status, state.body.clone()).into_response()
}

pub async fn iam_server(status: u16, body: &str) -> IamServer {
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
    IamServer { base, seen, tasks }
}

pub struct FixedAdc {
    pub token: Option<&'static str>,
}

impl AdcTokenSource for FixedAdc {
    fn access_token(&self, _scope: &str) -> Result<SecretString, AdcError> {
        match self.token {
            Some(token) => Ok(SecretString::from(token)),
            None => Err(AdcError::Adc {
                source: CredentialSourceError::MissingProject,
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GrantCall {
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

pub struct RecordingGrants {
    // None makes every grant refuse, so a test can assert how a failure surfaces
    pub token: Option<&'static str>,
    pub calls: Mutex<Vec<GrantCall>>,
}

impl RecordingGrants {
    pub fn new(token: &'static str) -> Self {
        Self {
            token: Some(token),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub fn refusing() -> Self {
        Self {
            token: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub fn calls(&self) -> Vec<GrantCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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
        let call = GrantCall::ClientSecret {
            tenant_id: tenant_id.to_owned(),
            client_id: client_id.to_owned(),
            scope: scope.to_owned(),
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        match self.token {
            Some(token) => Ok(SecretString::from(token)),
            None => Err(AzureGrantError::Build {
                kind: "client secret",
                message: "the test grant refuses".to_owned(),
            }),
        }
    }

    fn managed_identity(
        &self,
        client_id: &str,
        scope: &str,
    ) -> Result<SecretString, AzureGrantError> {
        let call = GrantCall::ManagedIdentity {
            client_id: client_id.to_owned(),
            scope: scope.to_owned(),
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        self.token.map_or_else(
            || {
                Err(AzureGrantError::ManagedIdentity {
                    source: betterlite_azure::azure_managed_identity::error::ManagedIdentityError::NoEndpoint,
                })
            },
            |token| Ok(SecretString::from(token)),
        )
    }

    fn default_credential(&self, scope: &str) -> Result<SecretString, AzureGrantError> {
        let token = self.token.ok_or_else(|| AzureGrantError::Build {
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

pub fn deps(
    runtime: Handle,
    adc: Arc<dyn AdcTokenSource>,
    azure: Arc<dyn AzureGrants>,
    aws: aws_credential_types::provider::SharedCredentialsProvider,
) -> CloudRedisAuthDeps {
    CloudRedisAuthDeps {
        runtime,
        http: proxied_client(),
        adc,
        azure,
        aws,
    }
}

pub fn sourced_settings(provider: Provider) -> Settings {
    Settings {
        provider: Some(provider),
        url: None,
        username: None,
        password: None,
        warnings: Vec::new(),
        environment_snapshot: BTreeMap::new(),
    }
}

pub fn gcp_provider() -> Provider {
    Provider::Gcp(GcpSettings {
        service_account: SecretString::from(
            "projects/-/serviceAccounts/sa@project.iam.gserviceaccount.com".to_owned(),
        ),
        ssl_ca_certs: None,
        token_ttl_seconds: 3300,
        token_cache: Arc::new(GcpTokenCache::default()),
    })
}

pub fn azure_settings(credential: AzureCredential) -> AzureSettings {
    AzureSettings {
        client_id: Some(SecretString::from("client".to_owned())),
        tenant_id: Some(SecretString::from("tenant".to_owned())),
        client_secret: Some(SecretString::from("secret".to_owned())),
        username: Some("redis-user".to_owned()),
        credential,
    }
}

pub fn azure_provider(credential: AzureCredential) -> Provider {
    Provider::Azure(azure_settings(credential))
}

pub fn aws_settings() -> AwsSettings {
    AwsSettings {
        user_name: "app".to_owned(),
        cache_name: "cache.example".to_owned(),
        region: "eu-west-1".to_owned(),
        serverless: false,
        token_ttl_seconds: 900,
    }
}

pub fn aws_provider() -> Provider {
    Provider::Aws(aws_settings())
}

pub fn static_aws_provider() -> aws_credential_types::provider::SharedCredentialsProvider {
    aws_credential_types::provider::SharedCredentialsProvider::new(
        aws_credential_types::Credentials::new(
            "AKIAFIXTURE",
            "fixture-secret",
            None,
            None,
            "betterlite-test",
        ),
    )
}

pub struct Reply {
    pub url_contains: &'static str,
    pub status: u16,
    pub body: &'static str,
}

pub struct FakeIo {
    pub replies: Mutex<VecDeque<Reply>>,
    pub requests: Mutex<Vec<String>>,
}

impl FakeIo {
    pub fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl betterlite_azure::azure_client_secret::transport::Transport for FakeIo {
    fn send(
        &self,
        request: &betterlite_azure::azure_client_secret::transport::HttpRequest,
    ) -> io::Result<betterlite_azure::azure_client_secret::transport::HttpReply> {
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
        Ok(
            betterlite_azure::azure_client_secret::transport::HttpReply {
                status: reply.status,
                body: SecretString::from(reply.body),
            },
        )
    }
}

impl betterlite_azure::azure_username_password::transport::Transport for FakeIo {
    fn send(
        &self,
        _request: &betterlite_azure::azure_username_password::transport::HttpRequest,
    ) -> io::Result<betterlite_azure::azure_client_secret::transport::HttpReply> {
        Err(io::Error::other("the fixture has no password endpoint"))
    }
}

impl betterlite_azure::oidc_exchange::transport::Transport for FakeIo {
    fn post_form(
        &self,
        _url: &str,
        _form: &[(String, SecretString)],
    ) -> io::Result<betterlite_azure::oidc_exchange::transport::HttpReply> {
        Err(io::Error::other("the fixture has no OIDC endpoint"))
    }
}

impl betterlite_azure::azure_managed_identity::imds::ImdsTransport for FakeIo {
    fn get(
        &self,
        _request: &betterlite_azure::azure_managed_identity::imds::ImdsRequest,
    ) -> io::Result<betterlite_azure::azure_managed_identity::imds::ImdsResponse> {
        Err(io::Error::other("the fixture has no IMDS endpoint"))
    }
}

impl betterlite_azure::azure_hosted_identity::request::HostedTransport for FakeIo {
    fn send(
        &self,
        _request: &betterlite_azure::azure_hosted_identity::request::HostedRequest,
    ) -> io::Result<betterlite_azure::azure_hosted_identity::request::HostedResponse> {
        Err(io::Error::other("the fixture has no hosted endpoint"))
    }
}

impl betterlite_azure::azure_arc_identity::ArcTransport for FakeIo {
    fn get(
        &self,
        _request: &betterlite_azure::azure_arc_identity::ArcRequest,
    ) -> io::Result<betterlite_azure::azure_arc_identity::ArcResponse> {
        Err(io::Error::other("the fixture has no Arc endpoint"))
    }
}
