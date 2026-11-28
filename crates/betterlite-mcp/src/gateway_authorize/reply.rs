mod tests;

use serde::Serialize;

use crate::gateway_dcr::oauth_error::OauthError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestView {
    pub base_url: String,
    pub path: String,
    pub query: Option<String>,
    pub cookies: Vec<(String, String)>,
}

impl RequestView {
    #[must_use]
    pub fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies
            .iter()
            .rev()
            .find(|(cookie_name, _value)| cookie_name == name)
            .map(|(_cookie_name, value)| value.as_str())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FlowCookie {
    pub name: String,
    pub value: String,
    pub max_age_seconds: i64,
    pub path: String,
    pub secure: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FlowCookieDeletion {
    pub name: String,
    pub path: String,
    pub secure: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectFlowState {
    Unscoped,
    Interactive,
    M2m,
    Stale,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ConnectFlowDescription {
    pub state: ConnectFlowState,
    pub client_origin: String,
    pub server_id: Option<String>,
    pub server_name: Option<String>,
    pub connected: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GatewayReply {
    Redirect {
        location: String,
        set_cookie: Option<Box<FlowCookie>>,
        delete_cookie: Option<Box<FlowCookieDeletion>>,
    },
    Html {
        body: String,
        headers: Vec<(String, String)>,
        set_cookie: Option<Box<FlowCookie>>,
        delete_cookie: Option<Box<FlowCookieDeletion>>,
    },
    Json {
        body: Box<ConnectFlowDescription>,
        headers: Vec<(String, String)>,
    },
    Error(OauthError),
}

impl GatewayReply {
    #[must_use]
    pub const fn redirect(location: String) -> Self {
        Self::Redirect {
            location,
            set_cookie: None,
            delete_cookie: None,
        }
    }
}

impl From<Result<Self, OauthError>> for GatewayReply {
    fn from(outcome: Result<Self, OauthError>) -> Self {
        match outcome {
            Ok(reply) => reply,
            Err(error) => Self::Error(error),
        }
    }
}

pub trait FlowTokenSource: Send + Sync {
    fn fresh_token(&self) -> String;
}

#[derive(Clone, Copy, Debug)]
pub struct ConsentPage<'page> {
    pub client_origin: &'page str,
    pub user_id: &'page str,
    pub teams: &'page [(String, String)],
    pub flow_handle: &'page str,
    pub complete_url: &'page str,
}

pub trait RenderConsentPage: Send + Sync {
    fn render(&self, page: ConsentPage<'_>) -> String;
}
