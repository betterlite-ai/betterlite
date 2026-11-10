use serde::Deserialize;
use serde::de::Error as _;
use serde_json::Value;

pub(crate) type Pair = (String, String);

#[derive(Deserialize)]
#[serde(try_from = "Value")]
pub(crate) struct Input {
    #[serde(default)]
    pub debug: Vec<DebugScenario>,
    #[serde(default)]
    pub diagnostics: Vec<DiagnosticScenario>,
    #[serde(default)]
    pub bodies: Vec<BodyScenario>,
}

impl TryFrom<Value> for Input {
    type Error = serde_json::Error;

    fn try_from(mut value: Value) -> Result<Self, Self::Error> {
        let operation = value
            .get("operation")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| serde_json::Error::custom("the helper input names no operation"))?;
        if let Value::Object(fields) = &mut value {
            fields.remove("operation");
        }
        if operation != "run" {
            return Err(serde_json::Error::custom(format!(
                "unknown b8194 operation: {operation}"
            )));
        }
        serde_json::from_value(value)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DebugScenario {
    #[serde(default)]
    pub headers: Vec<Pair>,
    #[serde(default)]
    pub oauth2_headers: Option<Vec<Pair>>,
    #[serde(default)]
    pub litellm_api_key: Option<String>,
    pub auth_resolution: String,
    #[serde(default)]
    pub server_url: Option<String>,
    #[serde(default)]
    pub server_auth_type: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DiagnosticScenario {
    #[serde(default)]
    pub records: Vec<Pair>,
    #[serde(default)]
    pub through_context: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BodyScenario {
    pub body: String,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub secrets: Vec<String>,
    #[serde(default)]
    pub url: Option<String>,
}
