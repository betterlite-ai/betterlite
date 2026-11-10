use serde::Deserialize;
use serde::de::Error as _;
use serde_json::Value;

pub(crate) type Pair = (String, String);

#[derive(Deserialize)]
#[serde(try_from = "Value")]
pub(crate) enum Input {
    DebugHeaders(DebugHeadersInput),
    AuthDiagnostics(DiagnosticsInput),
    WrapSend(WrapInput),
    MaskSecret(MaskInput),
    Preview(PreviewInput),
    RequestSecrets(RequestInput),
    DescribeUpstream(DescribeInput),
    DescribeFailure(FailureInput),
    Capture(CaptureInput),
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
        match operation.as_str() {
            "debug_headers" => Ok(Self::DebugHeaders(serde_json::from_value(value)?)),
            "auth_diagnostics" => Ok(Self::AuthDiagnostics(serde_json::from_value(value)?)),
            "wrap_send" => Ok(Self::WrapSend(serde_json::from_value(value)?)),
            "mask_secret" => Ok(Self::MaskSecret(serde_json::from_value(value)?)),
            "preview" => Ok(Self::Preview(serde_json::from_value(value)?)),
            "request_secrets" => Ok(Self::RequestSecrets(serde_json::from_value(value)?)),
            "describe_upstream" => Ok(Self::DescribeUpstream(serde_json::from_value(value)?)),
            "describe_failure" => Ok(Self::DescribeFailure(serde_json::from_value(value)?)),
            "capture" => Ok(Self::Capture(serde_json::from_value(value)?)),
            other => Err(serde_json::Error::custom(format!(
                "unknown b8194 operation: {other}"
            ))),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DebugHeadersInput {
    #[serde(default)]
    pub scenarios: Vec<DebugScenario>,
}

#[derive(Debug, Deserialize)]
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
    #[serde(default)]
    pub servers: Vec<ServerSpec>,
    #[serde(default)]
    pub mcp_servers: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServerSpec {
    pub name: String,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub auth_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DiagnosticsInput {
    #[serde(default)]
    pub scenarios: Vec<DiagnosticScenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DiagnosticScenario {
    #[serde(default)]
    pub records: Vec<Pair>,
    #[serde(default)]
    pub through_context: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WrapInput {
    #[serde(default)]
    pub scenarios: Vec<WrapScenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WrapScenario {
    #[serde(default)]
    pub debug_headers: Vec<Pair>,
    #[serde(default)]
    pub records: Vec<Pair>,
    #[serde(default = "default_method")]
    pub method: String,
}

fn default_method() -> String {
    "POST".to_owned()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MaskInput {
    #[serde(default)]
    pub values: Vec<Option<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreviewInput {
    #[serde(default)]
    pub scenarios: Vec<PreviewScenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreviewScenario {
    pub body: String,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub secrets: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestInput {
    #[serde(default)]
    pub scenarios: Vec<RequestSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RequestSpec {
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: Vec<Pair>,
    #[serde(default)]
    pub body: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponseSpec {
    pub status: u16,
    #[serde(default)]
    pub headers: Vec<Pair>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub captured: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DescribeInput {
    #[serde(default)]
    pub urls: Vec<String>,
    #[serde(default)]
    pub scenarios: Vec<DescribeScenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DescribeScenario {
    #[serde(default)]
    pub request: Option<RequestSpec>,
    pub response: ResponseSpec,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FailureInput {
    #[serde(default)]
    pub responses: Vec<ResponseSpec>,
    #[serde(default)]
    pub chains: Vec<Vec<Option<usize>>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CaptureInput {
    #[serde(default)]
    pub scenarios: Vec<CaptureScenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CaptureScenario {
    pub status: u16,
    #[serde(default)]
    pub headers: Vec<Pair>,
    pub body: String,
    #[serde(default)]
    pub request: Option<RequestSpec>,
    #[serde(default)]
    pub chunk_size: Option<usize>,
    #[serde(default)]
    pub fail_after_chunks: Option<usize>,
}
