use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Deserialize;

pub(crate) const MAX_DIALS: usize = 8;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderKind {
    Setup,
    Deferred,
    Error,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderErrorDocument {
    pub(crate) error_class: String,
    pub(crate) message: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProviderDocument {
    pub(crate) kind: ProviderKind,
    #[serde(default)]
    pub(crate) setup: Option<String>,
    #[serde(default)]
    pub(crate) synthetic: Option<Json>,
    #[serde(default)]
    pub(crate) error: Option<ProviderErrorDocument>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum DialDocument {
    Connect,
    Timeout {
        #[serde(default)]
        message: String,
    },
    Os {
        message: String,
    },
    #[serde(rename = "websocket")]
    WebSocket {
        message: String,
    },
    InvalidStatus {
        status_code: u16,
    },
    InvalidStatusCode {
        status_code: u16,
        message: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CloseErrorDocument {
    pub(crate) error_class: String,
    pub(crate) message: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) provider: Option<ProviderDocument>,
    pub(crate) model: String,
    pub(crate) url: String,
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<String>,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) ssl_verify: bool,
    pub(crate) max_message_size_bytes: usize,
    pub(crate) dials: Vec<DialDocument>,
    pub(crate) send_error: Option<String>,
    pub(crate) close_error: Option<CloseErrorDocument>,
    pub(crate) metadata: Option<IndexMap<String, Json>>,
    pub(crate) intent: Option<String>,
}

impl Input {
    pub(crate) fn check(&self) -> Result<(), String> {
        if self.provider.is_none() {
            return Err(String::from("the connection input names a provider"));
        }
        if self.url.is_empty() {
            return Err(String::from("the connection input names a url"));
        }
        if self.dials.len() > MAX_DIALS {
            return Err(String::from("the connection input is past its dial limit"));
        }
        Ok(())
    }
}
