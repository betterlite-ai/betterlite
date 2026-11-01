use crate::facts::ProviderErrorBody;
use crate::facts::presence::Field;

// the body fields openai.APIError and AzureOpenAIExceptionMapping read: top-level type/param/code plus the nested "error" object used for content-policy and structured-code detection
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AzureErrorBody {
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub param: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub error: Option<AzureErrorDetail>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AzureErrorDetail {
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub param: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub inner_error: Option<AzureInnerError>,
    #[serde(default)]
    pub innererror: Option<AzureInnerError>,
}

// only "code" is ever read off inner_error/innererror, so a present-but-empty object reproduces every outcome the isinstance(dict) check reaches for real Azure payloads
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AzureInnerError {
    #[serde(default)]
    pub code: Option<String>,
}

pub(super) fn typed_body(body: Option<&AzureErrorBody>) -> Option<ProviderErrorBody> {
    let fields = body?;
    let nested_message = fields
        .error
        .as_ref()
        .and_then(|error| error.message.clone());
    Some(ProviderErrorBody {
        message: fields.message.clone().map_or(Field::Missing, Field::Value),
        error: nested_message.map_or(Field::Missing, |message| {
            Field::Value(Box::new(ProviderErrorBody {
                message: Field::Value(message),
                error: Field::Missing,
                code: Field::Missing,
            }))
        }),
        code: Field::Missing,
    })
}

// body["error"]["code"], promoted to "content_policy_violation" when inner_error/innererror carries Azure's ResponsibleAIPolicyViolation code; falls back to the top-level body["code"] when body["error"] isn't present
pub(super) fn azure_error_code(body: Option<&AzureErrorBody>) -> Option<String> {
    let fields = body?;
    let Some(error) = &fields.error else {
        return fields.code.clone();
    };
    let mut code = error.code.clone();
    if code.as_deref() != Some("content_policy_violation") {
        // inner_error or innererror: inner_error wins whenever it is present, even without a matching code
        let inner = error.inner_error.as_ref().or(error.innererror.as_ref());
        if inner.and_then(|detail| detail.code.as_deref()) == Some("ResponsibleAIPolicyViolation") {
            code = Some("content_policy_violation".to_owned());
        }
    }
    code
}

// body["error"] when present, else body itself; matches AzureOpenAIExceptionMapping's own extract_azure_error
pub(super) fn content_policy_fields(
    body: Option<&AzureErrorBody>,
) -> (Option<String>, Option<String>) {
    match body {
        Some(AzureErrorBody {
            error: Some(error), ..
        }) => (error.kind.clone(), error.code.clone()),
        Some(plain) => (plain.kind.clone(), plain.code.clone()),
        None => (None, None),
    }
}
