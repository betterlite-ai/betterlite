use std::collections::BTreeMap;
use std::error::Error;
use std::future::Future;

use betterlite_prompts::arguments::json::Object;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::wire_compat::capabilities::policy::ClientCapabilities;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ElicitAction {
    Accept,
    Decline,
    Cancel,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ElicitContentValue {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    Text(String),
    Texts(Vec<String>),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ElicitResult {
    pub action: ElicitAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<BTreeMap<String, ElicitContentValue>>,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Object>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum UrlMode {
    #[default]
    Url,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FormMode {
    #[default]
    Form,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UrlElicitParams {
    #[serde(default)]
    pub mode: UrlMode,
    pub message: String,
    pub url: String,
    #[serde(default)]
    pub elicitation_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FormElicitParams {
    #[serde(default)]
    pub mode: FormMode,
    pub message: String,
    pub requested_schema: Object,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(untagged)]
pub enum ElicitRequestParams {
    Url(UrlElicitParams),
    Form(FormElicitParams),
}

pub trait DownstreamElicitSession {
    type Error: Error + Send + Sync + 'static;

    fn elicit(
        &self,
        params: &ElicitRequestParams,
    ) -> impl Future<Output = Result<ElicitResult, Self::Error>> + Send;
}

pub async fn handle_elicitation_request<Session>(
    params: &ElicitRequestParams,
    downstream_session: Option<&Session>,
    downstream_capabilities: Option<&ClientCapabilities>,
) -> ElicitResult
where
    Session: DownstreamElicitSession + Sync,
{
    let message = match params {
        ElicitRequestParams::Url(url) => url.message.as_str(),
        ElicitRequestParams::Form(form) => form.message.as_str(),
    };
    info!("MCP elicitation: received request message={message}");
    let Some(session) = downstream_session else {
        info!("MCP elicitation: no downstream session available, declining");
        return ElicitResult {
            action: ElicitAction::Decline,
            content: None,
            meta: None,
        };
    };
    if downstream_capabilities
        .is_some_and(|capabilities| !capabilities_support(params, capabilities))
    {
        info!("MCP elicitation: downstream client does not support this request");
        return ElicitResult {
            action: ElicitAction::Decline,
            content: None,
            meta: None,
        };
    }
    relay_to_downstream(params, session).await
}

const fn capabilities_support(
    params: &ElicitRequestParams,
    capabilities: &ClientCapabilities,
) -> bool {
    let Some(elicitation) = capabilities.elicitation.as_ref() else {
        return false;
    };
    match params {
        ElicitRequestParams::Url(_) => elicitation.url.is_some(),
        ElicitRequestParams::Form(_) => elicitation.form.is_some(),
    }
}

async fn relay_to_downstream<Session>(
    params: &ElicitRequestParams,
    session: &Session,
) -> ElicitResult
where
    Session: DownstreamElicitSession + Sync,
{
    match params {
        ElicitRequestParams::Url(_) => {
            info!("MCP elicitation: relaying URL mode to downstream");
        }
        ElicitRequestParams::Form(_) => {
            info!("MCP elicitation: relaying form mode to downstream");
        }
    }
    match session.elicit(params).await {
        Ok(result) => result,
        Err(error) => {
            warn!(%error, "MCP elicitation: failed to relay to downstream");
            ElicitResult {
                action: ElicitAction::Decline,
                content: None,
                meta: None,
            }
        }
    }
}

mod tests;
