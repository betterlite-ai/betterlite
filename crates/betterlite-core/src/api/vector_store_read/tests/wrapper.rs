use betterlite_models::decisions::Json;
use betterlite_openai::vector_stores::Headers;

use super::super::service::ReadConfig;
use crate::api::vector_store_http_create_read::config::CreateReadConfig;
use crate::api::vector_store_http_create_read::error::CapabilityError;
use crate::api::vector_store_http_write::config::{WriteConfig, WriteParams};
use crate::api::vector_store_http_write::error::{BaseLlmException, HeaderMap, UrlFailure};

// B010 hands B2485 a boxed config, so the wrapper must pass B527's capability check on to it
struct Refusing;

impl CreateReadConfig for Refusing {
    fn validate_create(&self) -> Result<(), CapabilityError> {
        Err(CapabilityError {
            message: String::from("no create"),
        })
    }

    fn provider(&self) -> &dyn WriteConfig {
        self
    }
}

impl WriteConfig for Refusing {
    fn validate_environment(&self, headers: Headers, _params: &WriteParams) -> Headers {
        headers
    }

    fn complete_url(&self, _params: &WriteParams) -> Result<String, UrlFailure> {
        Ok(String::from("https://mock.invalid/v1"))
    }

    fn transform_create_response(
        &self,
        _response: &http::Response<Vec<u8>>,
    ) -> Result<Json, BaseLlmException> {
        Ok(Json::Null)
    }

    fn error_class(
        &self,
        message: &str,
        status_code: u16,
        _headers: HeaderMap,
    ) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: message.to_owned(),
            headers: HeaderMap::new(),
            status_code_is_synthesized: false,
        }
    }
}

#[test]
fn read_config_passes_the_capability_check_through() {
    let config = ReadConfig(Box::new(Refusing));
    let refused = config
        .validate_create()
        .expect_err("the wrapped configuration refuses the create");
    assert_eq!(refused.message, "no create");
}
