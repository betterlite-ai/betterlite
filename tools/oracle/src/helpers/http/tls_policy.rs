use std::collections::BTreeMap;

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(crate) ssl_verify: Option<serde_json::Value>,
    pub(crate) http2: bool,
    pub(crate) environment: BTreeMap<String, String>,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mut verification = input
            .ssl_verify
            .clone()
            .unwrap_or(serde_json::Value::Bool(true));
        if let Some(value) = verification.as_str() {
            if value.eq_ignore_ascii_case("true") {
                verification = serde_json::Value::Bool(true);
            } else if value.eq_ignore_ascii_case("false") {
                verification = serde_json::Value::Bool(false);
            }
        }
        let http2 = input.http2
            || input
                .environment
                .get("LITELLM_HTTP2")
                .is_some_and(|value| value.eq_ignore_ascii_case("true"));
        let user_agent = input
            .environment
            .get("LITELLM_USER_AGENT")
            .cloned()
            .unwrap_or_else(|| "litellm/1.105.1".to_owned());
        Ok(
            serde_json::json!({"ssl_verify": verification, "http2": http2, "headers": {"User-Agent": user_agent}}),
        )
    }
}
