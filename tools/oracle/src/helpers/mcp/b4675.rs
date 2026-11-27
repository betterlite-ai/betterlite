use std::collections::{HashMap, VecDeque};
use std::fmt::Display;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

use betterlite_db::stored_cipher::DbCipher;
use betterlite_mcp::gateway_authorize::authorize::{
    AggregateAuthorize, AuthorizeRequest, NativeClientAuthorize, aggregate_authorize,
    native_client_authorize,
};
use betterlite_mcp::gateway_authorize::connect::{
    CompleteFlowRequest, ConnectFlowCall, ConnectPorts, FlowForm, complete_connect_flow,
    describe_connect_flow,
};
use betterlite_mcp::gateway_authorize::contract::native_client_auth_contract;
use betterlite_mcp::gateway_authorize::reply::{
    FlowCookie, FlowCookieDeletion, FlowTokenSource, GatewayReply, RenderConsentPage, RequestView,
};
use betterlite_mcp::gateway_dcr::contracts::GATEWAY_DCR_CLIENT_ID_PREFIX;
use betterlite_mcp::gateway_dcr::models::GatewayDcrClient;
use betterlite_mcp::gateway_dcr::oauth_error::OauthErrorBody;
use betterlite_mcp::gateway_dcr::ports::{
    ConsentTeam, LookupConsentTeams, LookupServerReachability, LookupVendorCredential,
    ReloadUserFailure, VendorCredentialState,
};
use betterlite_mcp::gateway_dcr::sealed::seal;
use betterlite_mcp::gateway_dcr::single_use::{
    ClaimAuthority, ClaimBackend, ClaimBackendError, ClaimClock, MemoryClaimStore, SingleUseGuard,
};
use betterlite_mcp::registry::servers::{RegistrySettings, ServerRegistry};
use betterlite_mcp::server_record::MCPServer;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::runtime::{Builder, Runtime};

const SALT: &str = "b4675-oracle-salt";
const BASE: &str = "https://proxy.test:8443";
const NO_CACHE: [(&str, &str); 2] = [("cache-control", "no-store"), ("pragma", "no-cache")];
const PAGE_HEADERS: [(&str, &str); 4] = [
    ("cache-control", "no-store"),
    ("content-security-policy", "frame-ancestors 'none'"),
    ("pragma", "no-cache"),
    ("x-frame-options", "DENY"),
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    clients: Vec<ClientSeed>,
    servers: Vec<Value>,
    steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClientSeed {
    redirect_uris: Vec<String>,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Aggregate,
    Native,
    Describe,
    Complete,
    Contract,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum VendorKind {
    Present,
    Absent,
    Unavailable,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum TeamsFailure {
    Unresolvable,
    Unavailable,
    Faulted,
    NoActiveKey,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TeamSeed {
    team_id: String,
    team_alias: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    name: String,
    kind: Kind,
    now: i64,
    path: String,
    query: Option<String>,
    cookie_from: i64,
    client: i64,
    client_id: String,
    redirect_uri: String,
    state: String,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    response_type: Option<String>,
    user: Option<String>,
    resource: Option<String>,
    tokens: Vec<String>,
    handle: String,
    delivery: Option<String>,
    team_id: Option<String>,
    decision: Option<String>,
    teams: Vec<TeamSeed>,
    teams_failure: Option<TeamsFailure>,
    vendor: VendorKind,
    reachable: bool,
    token_exchange: bool,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let cipher = cipher();
        let clients = client_ids(&input.clients, &cipher);
        let registry = registry(&input.servers)?;
        let runtime = Builder::new_current_thread().build().map_err(failure)?;
        let guard = guard();
        let mut captured: Vec<Option<String>> = Vec::new();
        let mut reports = Vec::new();
        for step in &input.steps {
            let sent = if step.cookie_from < 0 {
                None
            } else {
                captured[usize::try_from(step.cookie_from).unwrap_or_default()].clone()
            };
            if matches!(step.kind, Kind::Contract) {
                let contract = native_client_auth_contract(BASE, step.token_exchange);
                reports.push(contract_report(&step.name, &contract));
                captured.push(None);
                continue;
            }
            let (view, answers, tokens) = (
                view(step, sent),
                answers(step),
                ScriptedTokens::new(&step.tokens),
            );
            let client_id = if step.client < 0 {
                step.client_id.clone()
            } else {
                clients[usize::try_from(step.client).unwrap_or_default()].clone()
            };
            let request = AuthorizeRequest {
                client_id: &client_id,
                redirect_uri: &step.redirect_uri,
                state: &step.state,
                code_challenge: step.code_challenge.as_deref(),
                code_challenge_method: step.code_challenge_method.as_deref(),
                response_type: step.response_type.as_deref(),
            };
            let ports = ConnectPorts {
                registry: &registry,
                vendor: &answers,
                reachability: &answers,
            };
            let reply = match step.kind {
                Kind::Aggregate => aggregate_authorize(
                    &AggregateAuthorize {
                        view: &view,
                        request: &request,
                        session_user_id: step.user.as_deref(),
                        resource: step.resource.as_deref(),
                        now: step.now,
                    },
                    &registry,
                    &cipher,
                    &tokens,
                ),
                Kind::Native => runtime.block_on(native_client_authorize(
                    &NativeClientAuthorize {
                        view: &view,
                        request: &request,
                        session_user_id: step.user.as_deref(),
                        now: step.now,
                    },
                    &answers,
                    &answers,
                    &cipher,
                    &tokens,
                )),
                Kind::Describe => runtime.block_on(describe_connect_flow(
                    &ConnectFlowCall {
                        view: &view,
                        handle: &step.handle,
                        session_user_id: step.user.as_deref(),
                        now: step.now,
                    },
                    &ports,
                    &cipher,
                )),
                Kind::Complete | Kind::Contract => runtime.block_on(complete_connect_flow(
                    &CompleteFlowRequest {
                        call: ConnectFlowCall {
                            view: &view,
                            handle: &step.handle,
                            session_user_id: step.user.as_deref(),
                            now: step.now,
                        },
                        form: FlowForm {
                            delivery: step.delivery.as_deref(),
                            team_id: step.team_id.as_deref(),
                            decision: step.decision.as_deref(),
                        },
                    },
                    &ports,
                    &cipher,
                    &guard,
                    &tokens,
                )),
            };
            let (report, cookie) = step_report(&step.name, reply);
            reports.push(report);
            captured.push(cookie);
        }
        Ok(Value::Array(reports))
    }
}

struct Answers {
    vendor: VendorCredentialState,
    reachable: bool,
    teams: Result<Vec<ConsentTeam>, ReloadUserFailure>,
}

impl LookupVendorCredential for Answers {
    fn lookup(
        &self,
        _user_id: &str,
        _server_id: &str,
    ) -> impl Future<Output = VendorCredentialState> + Send {
        ready(self.vendor)
    }
}

impl LookupServerReachability for Answers {
    fn reachable(&self, _user_id: &str, _server_id: &str) -> impl Future<Output = bool> + Send {
        ready(self.reachable)
    }
}

impl LookupConsentTeams for Answers {
    fn consent_teams(
        &self,
        _user_id: &str,
    ) -> impl Future<Output = Result<Vec<ConsentTeam>, ReloadUserFailure>> + Send {
        ready(self.teams.clone())
    }
}

impl RenderConsentPage for Answers {
    fn render(
        &self,
        client_origin: &str,
        user_id: &str,
        teams: &[(String, String)],
        flow_handle: &str,
        complete_url: &str,
    ) -> String {
        let named: Vec<String> = teams
            .iter()
            .map(|(id, alias)| format!("[\"{id}\",\"{alias}\"]"))
            .collect();
        format!(
            "{{\"client_origin\":\"{client_origin}\",\"complete_url\":\"{complete_url}\",\"flow_handle\":\"{flow_handle}\",\"teams\":[{}],\"user_id\":\"{user_id}\"}}",
            named.join(",")
        )
    }
}

struct ScriptedTokens {
    values: Mutex<VecDeque<String>>,
}

impl ScriptedTokens {
    fn new(values: &[String]) -> Self {
        Self {
            values: Mutex::new(values.iter().cloned().collect()),
        }
    }
}

impl FlowTokenSource for ScriptedTokens {
    fn fresh_token(&self) -> String {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        values.pop_front().unwrap_or_default()
    }
}

struct ZeroClock;

impl ClaimClock for ZeroClock {
    fn now_seconds(&self) -> i64 {
        0
    }
}

struct NoBackend;

impl ClaimBackend for NoBackend {
    fn increment(
        &self,
        _key: &str,
        _ttl_seconds: i64,
    ) -> impl Future<Output = Result<u64, ClaimBackendError>> + Send {
        ready(Err(ClaimBackendError))
    }

    fn read(
        &self,
        _key: &str,
    ) -> impl Future<Output = Result<Option<u64>, ClaimBackendError>> + Send {
        ready(Err(ClaimBackendError))
    }
}

fn guard() -> SingleUseGuard<NoBackend> {
    SingleUseGuard::from(ClaimAuthority::Local(MemoryClaimStore {
        clock: Box::new(ZeroClock),
        entries: Mutex::new(HashMap::new()),
    }))
}

fn cipher() -> DbCipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(SALT)),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

fn client_ids(seeds: &[ClientSeed], cipher: &DbCipher) -> Vec<String> {
    seeds
        .iter()
        .map(|seed| {
            let client = GatewayDcrClient {
                redirect_uris: seed.redirect_uris.clone(),
                iat: 0,
            };
            seal(GATEWAY_DCR_CLIENT_ID_PREFIX, &client, cipher).unwrap_or_default()
        })
        .collect()
}

fn registry(servers: &[Value]) -> Result<ServerRegistry, Box<HelperError>> {
    let records = servers
        .iter()
        .map(|raw| serde_json::from_value::<MCPServer>(raw.clone()).map_err(failure))
        .collect::<Result<Vec<_>, Box<HelperError>>>()?;
    Ok(ServerRegistry::new(
        RegistrySettings {
            public_mcp_servers: Vec::new(),
            public_hub_strict: false,
            internal_ip_ranges: None,
        },
        Vec::new(),
        records,
    ))
}

fn answers(step: &Step) -> Answers {
    let vendor = match step.vendor {
        VendorKind::Present => VendorCredentialState::Present,
        VendorKind::Absent => VendorCredentialState::Absent,
        VendorKind::Unavailable => VendorCredentialState::Unavailable,
    };
    let teams = match step.teams_failure {
        Some(TeamsFailure::Unresolvable) => Err(ReloadUserFailure::Unresolvable),
        Some(TeamsFailure::Unavailable) => Err(ReloadUserFailure::Unavailable),
        Some(TeamsFailure::Faulted) => Err(ReloadUserFailure::Faulted),
        Some(TeamsFailure::NoActiveKey) => Err(ReloadUserFailure::NoActiveKey),
        None => Ok(step
            .teams
            .iter()
            .map(|team| ConsentTeam {
                team_id: team.team_id.clone(),
                team_alias: team.team_alias.clone(),
            })
            .collect()),
    };
    Answers {
        vendor,
        reachable: step.reachable,
        teams,
    }
}

fn view(step: &Step, cookie: Option<String>) -> RequestView {
    let cookies = cookie
        .and_then(|pair| {
            pair.split_once('=')
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
        })
        .into_iter()
        .collect();
    RequestView {
        base_url: BASE.to_owned(),
        path: step.path.clone(),
        query: step.query.clone(),
        cookies,
    }
}

fn step_report(name: &str, reply: GatewayReply) -> (Value, Option<String>) {
    let mut captured = None;
    let (status, location, headers, body, cookie) = match reply {
        GatewayReply::Redirect {
            location,
            set_cookie,
            delete_cookie,
        } => {
            captured = set_cookie
                .as_ref()
                .map(|cookie| format!("{}={}", cookie.name, cookie.value));
            (
                303,
                Value::String(redact(&location)),
                Vec::new(),
                Value::Null,
                cookie_text(set_cookie, delete_cookie),
            )
        }
        GatewayReply::Html {
            body,
            headers,
            set_cookie,
            delete_cookie,
        } => {
            captured = set_cookie
                .as_ref()
                .map(|cookie| format!("{}={}", cookie.name, cookie.value));
            (
                200,
                Value::Null,
                headers,
                Value::String(redact(&body)),
                cookie_text(set_cookie, delete_cookie),
            )
        }
        GatewayReply::Json { body, headers } => (
            200,
            Value::Null,
            headers,
            Value::String(serde_json::to_string(&body).unwrap_or_default()),
            Value::Null,
        ),
        GatewayReply::Error(error) => (
            error.status,
            Value::Null,
            NO_CACHE
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
            Value::String(
                serde_json::to_string(&OauthErrorBody {
                    error: error.error,
                    error_description: &error.description,
                })
                .unwrap_or_default(),
            ),
            Value::Null,
        ),
    };
    let report = json!({
        "name": name,
        "status": status,
        "location": location,
        "headers": header_pairs(&headers),
        "body": body,
        "cookie": cookie,
    });
    (report, captured)
}

fn contract_report(name: &str, contract: &impl serde::Serialize) -> Value {
    json!({
        "name": name,
        "status": 200,
        "location": Value::Null,
        "headers": Value::Array(Vec::new()),
        "body": serde_json::to_string(contract).unwrap_or_default(),
        "cookie": Value::Null,
    })
}

fn header_pairs(headers: &[(String, String)]) -> Value {
    let mut pairs = Vec::new();
    for name in PAGE_HEADERS.iter().map(|(name, _value)| *name) {
        if let Some((_found, value)) = headers
            .iter()
            .find(|(header, _value)| header.eq_ignore_ascii_case(name))
        {
            pairs.push(json!([name, value]));
        }
    }
    Value::Array(pairs)
}

fn cookie_text(set: Option<Box<FlowCookie>>, delete: Option<Box<FlowCookieDeletion>>) -> Value {
    if let Some(cookie) = set {
        return Value::String(format!(
            "{}|{}|{}|{}|true|lax",
            cookie.name, cookie.max_age_seconds, cookie.path, cookie.secure
        ));
    }
    if let Some(cookie) = delete {
        return Value::String(format!(
            "{}|0|{}|{}|true|lax",
            cookie.name, cookie.path, cookie.secure
        ));
    }
    Value::Null
}

fn redact(text: &str) -> String {
    let Some((head, rest)) = text.split_once("code=") else {
        return text.to_owned();
    };
    let tail = rest.split_once('&').map_or("", |(_code, rest)| rest);
    let mut redacted = format!("{head}code=<code>");
    if !tail.is_empty() {
        redacted.push('&');
        redacted.push_str(tail);
    }
    redacted
}

fn failure<E>(error: E) -> Box<HelperError>
where
    E: Display,
{
    Box::new(HelperError {
        class: "HelperFailure".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
