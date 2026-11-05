pub(crate) struct Op;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub proxy_type: Option<String>,
    pub message: String,
    pub class: Option<betterlite_error::normalize::class::Class>,
    pub status_code: Option<u16>,
    pub rate_limit_type: Option<betterlite_error::rate_limit::RateLimitDimension>,
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
        let normalizer = betterlite_error::normalize::Normalizer::new().map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "NormalizationError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        let facts = betterlite_error::normalize::ErrorFacts {
            proxy_type: input.proxy_type.clone(),
            message: input.message.clone(),
            class: input.class.unwrap_or_default(),
            status_code: input.status_code,
            rate_limit_type: input.rate_limit_type,
        };
        Ok(serde_json::json!({"result": normalizer.normalize(Some(&facts))}))
    }
}
