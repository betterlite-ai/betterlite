use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_server::password_policy::breach::BreachChecker;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::sleep;

use super::failure;
use crate::helpers::HelperError;

// A reply that outlasts the checker's deadline is how the mock reports a timeout.
const TIMEOUT_REPLY_DELAY: Duration = Duration::from_secs(5);
const CHECK_DEADLINE: Duration = Duration::from_millis(200);
const UNREACHABLE_BASE: &str = "http://127.0.0.1:1/range";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    #[serde(default = "default_status")]
    status: u16,
    #[serde(default)]
    body: String,
    #[serde(default)]
    timeout: bool,
}

const fn default_status() -> u16 {
    200
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Hibp {
    #[serde(default)]
    replies: BTreeMap<String, Reply>,
    #[serde(default)]
    unreachable: bool,
}

#[derive(Clone)]
struct Mock {
    replies: Arc<BTreeMap<String, (u16, String, bool)>>,
    seen: Arc<Mutex<Vec<Value>>>,
}

pub(super) struct Fixture {
    pub(super) checker: BreachChecker,
    seen: Arc<Mutex<Vec<Value>>>,
    _server: JoinSet<()>,
}

impl Fixture {
    // Concurrent lookups arrive in no fixed order, so the record is sorted by prefix.
    pub(super) fn requests(&self) -> Value {
        let mut seen = self
            .seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        seen.sort_by_key(Value::to_string);
        Value::Array(seen)
    }
}

async fn range(
    State(mock): State<Mock>,
    Path(prefix): Path<String>,
    headers: HeaderMap,
) -> (StatusCode, String) {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    mock.seen
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(json!({
            "path": format!("/range/{prefix}"),
            "add_padding": header("add-padding"),
            "user_agent": header("user-agent"),
        }));
    let (status, body, timeout) =
        mock.replies
            .get(&prefix)
            .cloned()
            .unwrap_or((200, String::new(), false));
    if timeout {
        sleep(TIMEOUT_REPLY_DELAY).await;
    }
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        body,
    )
}

fn client() -> Result<ProxiedClient, Box<HelperError>> {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
        .map_err(|error| failure(&error))?;
    ProxiedClient::outbound(&Arc::new(policy)).map_err(|error| failure(&error))
}

async fn serve(mock: Mock) -> Result<(SocketAddr, JoinSet<()>), Box<HelperError>> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|error| failure(&error))?;
    let address = listener.local_addr().map_err(|error| failure(&error))?;
    let application = Router::new()
        .route("/range/{prefix}", get(range))
        .with_state(mock);
    let mut server = JoinSet::new();
    server.spawn(async move {
        // The server only stops when the fixture drops and aborts it.
        let _served = axum::serve(listener, application).await;
    });
    Ok((address, server))
}

pub(super) async fn start(hibp: &Hibp, enabled: bool) -> Result<Fixture, Box<HelperError>> {
    let replies = hibp
        .replies
        .iter()
        .map(|(prefix, reply)| {
            (
                prefix.clone(),
                (reply.status, reply.body.clone(), reply.timeout),
            )
        })
        .collect();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mock = Mock {
        replies: Arc::new(replies),
        seen: Arc::clone(&seen),
    };
    let (address, server) = serve(mock).await?;
    let base = if hibp.unreachable {
        UNREACHABLE_BASE.to_owned()
    } else {
        format!("http://{address}/range")
    };
    Ok(Fixture {
        checker: BreachChecker {
            client: client()?,
            base,
            timeout: CHECK_DEADLINE,
            enabled,
        },
        seen,
        _server: server,
    })
}
