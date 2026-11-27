use std::time::Duration;

use betterlite_bedrock::image_call::plan::ImageEnvironment;
use betterlite_bedrock::image_models::params::Params;
use secrecy::SecretString;

use crate::bedrock_image_dispatch::call::GenerateOperation;
use crate::bedrock_image_dispatch::test_support::{NOW_SECONDS, instant};

// The operation the caller resolved: a field left `None` or `false` is what the arm omits
pub(crate) struct GenerateFixture {
    pub(crate) model: Option<String>,
    pub(crate) prompt: String,
    pub(crate) optional_params: Params,
    pub(crate) extra_headers: Option<Vec<(String, String)>>,
    pub(crate) api_base: Option<String>,
    pub(crate) api_key: Option<SecretString>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) aimg_generation: bool,
}

impl GenerateFixture {
    pub(crate) fn new(model: Option<&str>, prompt: &str, optional_params: Params) -> Self {
        Self {
            model: model.map(str::to_owned),
            prompt: prompt.to_owned(),
            optional_params,
            extra_headers: None,
            api_base: None,
            api_key: None,
            timeout: None,
            aimg_generation: false,
        }
    }

    pub(crate) fn operation(&self) -> GenerateOperation<'_> {
        GenerateOperation {
            model: self.model.as_deref(),
            prompt: &self.prompt,
            optional_params: self.optional_params.clone(),
            extra_headers: self.extra_headers.as_deref(),
            api_base: self.api_base.as_deref(),
            api_key: self.api_key.as_ref(),
            timeout: self.timeout,
            aimg_generation: self.aimg_generation,
            environment: ImageEnvironment::default(),
            now: instant(NOW_SECONDS).expect("the fixture instant is after the Unix epoch"),
            created: NOW_SECONDS,
        }
    }
}
