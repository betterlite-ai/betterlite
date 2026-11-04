pub(crate) struct Op;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub proxy_type: Option<String>,
    pub message: String,
    pub class: Option<String>,
    pub status_code: Option<u16>,
    pub rate_limit_type: Option<String>,
    #[serde(default = "default_error_present")]
    pub error_present: bool,
}

const fn default_error_present() -> bool {
    true
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        if !input.error_present {
            return Ok(serde_json::json!({"result": null}));
        }
        let facts = betterlite_error::normalize::ErrorFacts {
            proxy_type: input.proxy_type.clone(),
            message: input.message.clone(),
            class: input.class.clone(),
            status_code: input.status_code,
            rate_limit_type: input.rate_limit_type.clone(),
        };
        Ok(serde_json::json!({"result": betterlite_error::normalize::normalize_error(&facts)}))
    }
}
