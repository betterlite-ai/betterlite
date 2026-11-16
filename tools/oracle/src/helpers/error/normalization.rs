use betterlite_error::normalize::class::Class;
use betterlite_error::normalize::{ErrorFacts, Normalizer};
use betterlite_error::rate_limit::RateLimitDimension;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

pub(crate) struct Op;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub proxy_type: Option<String>,
    pub message: String,
    pub class: Option<Class>,
    pub status_code: Option<u16>,
    pub rate_limit_type: Option<RateLimitDimension>,
    #[serde(default = "default_error_present")]
    pub error_present: bool,
}

const fn default_error_present() -> bool {
    true
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        if !input.error_present {
            return Ok(serde_json::json!({"result": null}));
        }
        let normalizer = Normalizer::new().map_err(|error| {
            Box::new(HelperError {
                class: "NormalizationError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        let facts = ErrorFacts {
            proxy_type: input.proxy_type.clone(),
            message: input.message.clone(),
            class: input.class.unwrap_or_default(),
            status_code: input.status_code,
            rate_limit_type: input.rate_limit_type,
        };
        Ok(serde_json::json!({"result": normalizer.normalize(Some(&facts))}))
    }
}
