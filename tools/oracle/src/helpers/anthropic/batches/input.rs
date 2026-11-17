use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    AuthHeaders(AuthCase),
    CreateRequests(CreateCase),
    ErrorCounts(ErrorCase),
    Lifecycle(BodiesCase),
    ResultResponse(BodiesCase),
    Retrieve(RetrieveCase),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BodiesCase {
    pub(crate) now: i64,
    pub(crate) bodies: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateCase {
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) url_bases: Vec<Option<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorCase {
    pub(crate) now: i64,
    pub(crate) bodies: Vec<String>,
    pub(crate) errors: Vec<ErrorScenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ErrorScenario {
    pub(crate) message: String,
    pub(crate) status: u16,
    pub(crate) headers: Option<Vec<(String, String)>>,
    pub(crate) headers_object: Option<Vec<(String, String)>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RetrieveCase {
    pub(crate) now: i64,
    pub(crate) scenarios: Vec<RetrieveScenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RetrieveScenario {
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) batch_id: String,
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<String>,
    #[serde(default)]
    pub(crate) sync: bool,
    pub(crate) status: u16,
    #[serde(default)]
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthCase {
    pub(crate) scenarios: Vec<AuthScenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthScenario {
    #[serde(default)]
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) api_key: Option<String>,
    pub(crate) api_base: Option<String>,
    pub(crate) params_api_base: Option<String>,
    #[serde(default)]
    pub(crate) caller_headers: Vec<(String, String)>,
    pub(crate) expected_auth: Option<String>,
}
