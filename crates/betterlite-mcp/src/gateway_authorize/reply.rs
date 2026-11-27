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

#[derive(Clone, Debug, PartialEq)]
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
    pub fn redirect(location: String) -> Self {
        Self::Redirect {
            location,
            set_cookie: None,
            delete_cookie: None,
        }
    }

    #[must_use]
    pub fn error(error: OauthError) -> Self {
        Self::Error(error)
    }
}

pub trait FlowTokenSource: Send + Sync {
    fn fresh_token(&self) -> String;
}

pub trait RenderConsentPage: Send + Sync {
    fn render(
        &self,
        client_origin: &str,
        user_id: &str,
        teams: &[(String, String)],
        flow_handle: &str,
        complete_url: &str,
    ) -> String;
}
