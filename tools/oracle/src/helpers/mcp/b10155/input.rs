use std::collections::BTreeMap;

use serde::Deserialize;

pub(crate) type HeaderMap = BTreeMap<String, BTreeMap<String, String>>;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServerSpec {
    pub(crate) alias: Option<String>,
    pub(crate) server_name: Option<String>,
    pub(crate) access_groups: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Call {
    SanitizeAlias {
        alias: String,
    },
    Lookup {
        headers: HeaderMap,
        alias: Option<String>,
        server_name: Option<String>,
        access_groups: Vec<String>,
    },
    PerServerAuthorization {
        server: ServerSpec,
        server_auth_headers: Option<HeaderMap>,
    },
    PassthroughAuthorization {
        server: ServerSpec,
        server_auth_headers: Option<HeaderMap>,
        oauth2_headers: Option<BTreeMap<String, String>>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) calls: Vec<Call>,
}
