use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use betterlite_db::mcp_stored_contracts::MCPAuth;
use betterlite_mcp::idp_token_exchange::{PrerequisiteSwitch, TokenExchangePrerequisites};
use betterlite_mcp::oauth_metadata_cache::OAuthMetadataCache;
use betterlite_mcp::registry::servers::{RegistrySettings, ServerRegistry, SharedRegistry};
use betterlite_mcp::server_record::MCPServer;
use http::request::Parts;
use tokio::sync::RwLock;
use tokio::task::yield_now;
use tokio::time::Instant;
use tower::ServiceExt as _;

use crate::mcp::oauth_protected::upstream_fetch::{BodyOutcome, parse_metadata_body};
use crate::mcp::oauth_protected::{
    GatewayFacts, GetFailure, JsonObject, ProtectedRoot, RequestFacts, UpstreamGet, UpstreamReply,
};

pub(crate) const BASE_URL: &str = "http://litellm.internal";
pub(crate) const PASS_THROUGH_URL: &str = "https://upstream.example.test/mcp";
pub(crate) const PASS_THROUGH_HOST: &str =
    "https://upstream.example.test/.well-known/oauth-protected-resource";
pub(crate) const PASS_THROUGH_PATH: &str =
    "https://upstream.example.test/.well-known/oauth-protected-resource/mcp";
pub(crate) const UPSTREAM_DOCUMENT: &str = r#"{"resource":"https://upstream.example.test/mcp","authorization_servers":["https://idp.example.test"],"scopes_supported":["mcp"]}"#;
pub(crate) const ROOT_DOCUMENT: &str = r#"{"resource":"http://litellm.internal","authorization_servers":["http://litellm.internal/mcp"],"scopes_supported":[]}"#;
pub(crate) const AGGREGATE_RESOURCE_DOCUMENT: &str = r#"{"authorization_servers":["http://litellm.internal/mcp"],"resource":"http://litellm.internal/mcp","scopes_supported":[]}"#;
pub(crate) const AGGREGATE_AUTH_SERVER_DOCUMENT: &str = r#"{"issuer":"http://litellm.internal/mcp","authorization_endpoint":"http://litellm.internal/authorize/mcp-session","token_endpoint":"http://litellm.internal/token","introspection_endpoint":"http://litellm.internal/introspect","registration_endpoint":"http://litellm.internal/register","response_types_supported":["code"],"scopes_supported":[],"grant_types_supported":["authorization_code","refresh_token"],"code_challenge_methods_supported":["S256"],"token_endpoint_auth_methods_supported":["none","client_secret_post"]}"#;
pub(crate) const AGGREGATE_AUTH_SERVER_WITH_EXCHANGE_DOCUMENT: &str = r#"{"issuer":"http://litellm.internal/mcp","authorization_endpoint":"http://litellm.internal/authorize/mcp-session","token_endpoint":"http://litellm.internal/token","introspection_endpoint":"http://litellm.internal/introspect","registration_endpoint":"http://litellm.internal/register","response_types_supported":["code"],"scopes_supported":[],"grant_types_supported":["authorization_code","refresh_token","urn:ietf:params:oauth:grant-type:token-exchange"],"code_challenge_methods_supported":["S256"],"token_endpoint_auth_methods_supported":["none","client_secret_post"]}"#;
pub(crate) const GATEWAY_STANDARD_DOCUMENT: &str = r#"{"authorization_servers":["http://litellm.internal/mcp"],"resource":"http://litellm.internal/mcp/gateway","scopes_supported":[]}"#;
pub(crate) const GATEWAY_LEGACY_DOCUMENT: &str = r#"{"authorization_servers":["http://litellm.internal/mcp"],"resource":"http://litellm.internal/gateway/mcp","scopes_supported":[]}"#;

pub(crate) fn document(raw: &str) -> JsonObject {
    match parse_metadata_body(raw.as_bytes()) {
        BodyOutcome::Object(parsed) => parsed,
        BodyOutcome::NotObject => panic!("the fixture is a JSON object"),
        BodyOutcome::NotJson(error) => panic!("the fixture is JSON: {error}"),
    }
}

pub(crate) fn rendered(json: &JsonObject) -> String {
    serde_json::to_string(json).expect("the document serializes")
}

pub(crate) struct FakeUpstream {
    replies: BTreeMap<String, Result<UpstreamReply, GetFailure>>,
    calls: Arc<Mutex<Vec<(String, String)>>>,
    invalidate: Option<(OAuthMetadataCache, String)>,
}

impl FakeUpstream {
    pub(crate) fn new(replies: Vec<(String, Result<UpstreamReply, GetFailure>)>) -> Self {
        Self {
            replies: replies.into_iter().collect(),
            calls: Arc::new(Mutex::new(Vec::new())),
            invalidate: None,
        }
    }

    pub(crate) fn invalidating(
        mut self,
        invalidation: Option<(OAuthMetadataCache, String)>,
    ) -> Self {
        self.invalidate = invalidation;
        self
    }

    pub(crate) fn calls(&self) -> Vec<(String, String)> {
        self.calls.lock().expect("the fake upstream lock").clone()
    }
}

impl UpstreamGet for FakeUpstream {
    fn get(
        &self,
        url: &str,
        accept: &str,
    ) -> impl Future<Output = Result<UpstreamReply, GetFailure>> + Send {
        self.calls
            .lock()
            .expect("the fake upstream lock")
            .push((url.to_owned(), accept.to_owned()));
        if let Some((cache, server_id)) = &self.invalidate {
            cache.invalidate(server_id);
        }
        let reply = self.replies.get(url).cloned().unwrap_or_else(|| {
            Err(GetFailure::Other(format!(
                "the fake has no reply for {url}"
            )))
        });
        async move {
            yield_now().await;
            reply
        }
    }
}

pub(crate) struct FakeRequest {
    pub(crate) client_ip: Option<String>,
}

impl RequestFacts for FakeRequest {
    fn request_base_url(&self, _request: &Parts) -> String {
        BASE_URL.to_owned()
    }

    fn client_ip(&self, _request: &Parts) -> Option<String> {
        self.client_ip.clone()
    }
}

pub(crate) struct FakeGateway {
    pub(crate) issuers: Vec<String>,
    pub(crate) token_exchange: TokenExchangePrerequisites,
}

impl GatewayFacts for FakeGateway {
    fn jwt_issuers(&self) -> Vec<String> {
        self.issuers.clone()
    }

    fn token_exchange(&self) -> TokenExchangePrerequisites {
        self.token_exchange
    }
}

pub(crate) const fn exchange_available() -> TokenExchangePrerequisites {
    TokenExchangePrerequisites {
        jwt_auth_enabled: PrerequisiteSwitch(true),
        has_database: PrerequisiteSwitch(true),
        maps_jwts_to_virtual_keys: PrerequisiteSwitch(false),
    }
}

pub(crate) const fn exchange_unavailable() -> TokenExchangePrerequisites {
    TokenExchangePrerequisites {
        jwt_auth_enabled: PrerequisiteSwitch(false),
        has_database: PrerequisiteSwitch(true),
        maps_jwts_to_virtual_keys: PrerequisiteSwitch(false),
    }
}

pub(crate) fn record(raw: &str) -> MCPServer {
    serde_json::from_str(raw).expect("the MCP server record decodes")
}

pub(crate) fn oauth_server(server_id: &str, name: &str) -> MCPServer {
    let raw = format!(r#"{{"server_id":"{server_id}","name":"{name}","transport":"http"}}"#);
    let mut server = record(&raw);
    server.fields.auth_type = Some(MCPAuth::Oauth2);
    server
}

pub(crate) fn gateway_server() -> MCPServer {
    oauth_server("srv-gateway", "gateway")
}

pub(crate) fn relay_server() -> MCPServer {
    let mut server = oauth_server("srv-relay", "test-oauth-1");
    server.fields.per_server_oauth_discovery = true;
    server
}

pub(crate) fn passthrough_server() -> MCPServer {
    let mut server = oauth_server("srv-passthrough", "upstream");
    server.fields.auth_type = Some(MCPAuth::TruePassthrough);
    server.fields.url = Some(PASS_THROUGH_URL.to_owned());
    server
}

pub(crate) fn delegate_server() -> MCPServer {
    let mut server = oauth_server("srv-delegate", "delegate");
    server.fields.auth_type = Some(MCPAuth::OauthDelegate);
    server.fields.url = Some(PASS_THROUGH_URL.to_owned());
    server
}

pub(crate) fn legacy_passthrough_server() -> MCPServer {
    let mut server = oauth_server("srv-legacy", "legacy");
    server.fields.auth_type = None;
    server.fields.oauth_passthrough = true;
    server.fields.extra_headers = Some(vec!["Authorization".to_owned()]);
    server.fields.url = Some(PASS_THROUGH_URL.to_owned());
    server
}

pub(crate) fn dcr_bridge_server() -> MCPServer {
    let mut server = oauth_server("srv-bridge", "bridge");
    server.fields.auth_type = Some(MCPAuth::TruePassthrough);
    server.fields.dcr_bridge = Some(true);
    server.fields.url = Some(PASS_THROUGH_URL.to_owned());
    server.fields.scopes = Some(vec!["read".to_owned()]);
    server
}

pub(crate) fn obo_server() -> MCPServer {
    let mut server = oauth_server("srv-obo", "obo");
    server.fields.auth_type = Some(MCPAuth::Oauth2TokenExchange);
    server.fields.scopes = Some(vec!["read".to_owned()]);
    server
}

pub(crate) fn keyed_server() -> MCPServer {
    let mut server = oauth_server("srv-keyed", "keyed");
    server.fields.auth_type = Some(MCPAuth::ApiKey);
    server.fields.extra_headers = Some(vec!["Authorization".to_owned()]);
    server
}

#[derive(Default)]
pub(crate) struct Harness {
    servers: Vec<MCPServer>,
    replies: Vec<(String, Result<UpstreamReply, GetFailure>)>,
    issuers: Vec<String>,
    token_exchange: Option<TokenExchangePrerequisites>,
    suffix: String,
    client_ip: Option<String>,
    cache: OAuthMetadataCache,
    invalidate_server: Option<String>,
}

impl Harness {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn servers(mut self, servers: Vec<MCPServer>) -> Self {
        self.servers = servers;
        self
    }

    pub(crate) fn reply(mut self, url: &str, body: &str) -> Self {
        self.replies.push((url.to_owned(), answer(200, body)));
        self
    }

    pub(crate) fn status(self, url: &str, status: u16) -> Self {
        self.status_body(url, status, "")
    }

    pub(crate) fn status_body(mut self, url: &str, status: u16, body: &str) -> Self {
        self.replies.push((url.to_owned(), answer(status, body)));
        self
    }

    pub(crate) fn transport_failure(mut self, url: &str, message: &str) -> Self {
        self.replies.push((
            url.to_owned(),
            Err(GetFailure::Transport(message.to_owned())),
        ));
        self
    }

    pub(crate) fn other_failure(mut self, url: &str, message: &str) -> Self {
        self.replies
            .push((url.to_owned(), Err(GetFailure::Other(message.to_owned()))));
        self
    }

    pub(crate) fn issuers(mut self, issuers: &[&str]) -> Self {
        self.issuers = issuers.iter().map(|issuer| String::from(*issuer)).collect();
        self
    }

    pub(crate) const fn token_exchange(
        mut self,
        prerequisites: TokenExchangePrerequisites,
    ) -> Self {
        self.token_exchange = Some(prerequisites);
        self
    }

    pub(crate) fn suffix(mut self, suffix: &str) -> Self {
        suffix.clone_into(&mut self.suffix);
        self
    }

    pub(crate) fn client_ip(mut self, client_ip: &str) -> Self {
        self.client_ip = Some(client_ip.to_owned());
        self
    }

    pub(crate) fn invalidate_on_fetch(mut self, server_id: &str) -> Self {
        self.invalidate_server = Some(server_id.to_owned());
        self
    }

    pub(crate) fn root(&self) -> ProtectedRoot<FakeUpstream, FakeRequest, FakeGateway> {
        let invalidation = self
            .invalidate_server
            .as_ref()
            .map(|server_id| (self.cache.clone(), server_id.clone()));
        let upstream = FakeUpstream::new(self.replies.clone()).invalidating(invalidation);
        ProtectedRoot {
            registry: registry(self.servers.clone()),
            cache: self.cache.clone(),
            upstream,
            request: FakeRequest {
                client_ip: self.client_ip.clone(),
            },
            gateway: FakeGateway {
                issuers: self.issuers.clone(),
                token_exchange: self.token_exchange.unwrap_or_else(exchange_unavailable),
            },
            well_known_suffix: self.suffix.clone(),
            clock: Instant::now,
        }
    }

    pub(crate) fn router(&self) -> Router {
        self.root().router()
    }
}

fn answer(status: u16, body: &str) -> Result<UpstreamReply, GetFailure> {
    Ok(UpstreamReply {
        status,
        body: body.as_bytes().to_vec(),
    })
}

pub(crate) fn registry(servers: Vec<MCPServer>) -> SharedRegistry {
    let settings = RegistrySettings {
        public_mcp_servers: Vec::new(),
        public_hub_strict: false,
        internal_ip_ranges: None,
    };
    Arc::new(RwLock::new(ServerRegistry::new(
        settings,
        servers,
        Vec::new(),
    )))
}

pub(crate) struct Reply {
    pub(crate) status: StatusCode,
    pub(crate) body: String,
}

pub(crate) async fn send_get(router: &Router, path: &str) -> Reply {
    let request = Request::get(path)
        .body(Body::empty())
        .expect("the request builds");
    let response = router
        .clone()
        .oneshot(request)
        .await
        .expect("the router answers");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the reply body reads");
    Reply {
        status,
        body: String::from_utf8(bytes.to_vec()).expect("the reply body is UTF-8"),
    }
}
