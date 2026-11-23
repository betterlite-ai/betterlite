use betterlite_config::sections::request_headers::{Settings, UserHeaderMapping, UserHeaderRole};
use betterlite_logging::end_user::extraction::{IdSettings, request_end_user_id};
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use http::{HeaderMap, HeaderName, HeaderValue};
use serde::Deserialize;
use serde_json::json;

mod tests;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    user_header_mappings: Vec<Mapping>,
    user_header_name: Option<String>,
    validate_end_user_id_in_db: bool,
    requests: Vec<Request>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Mapping {
    header_name: String,
    litellm_user_role: UserHeaderRole,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    body: JsonObject,
    headers: Option<Vec<(String, String)>>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let request_headers = Settings {
            user_header_mappings: input
                .user_header_mappings
                .iter()
                .map(|mapping| UserHeaderMapping {
                    header_name: mapping.header_name.clone(),
                    litellm_user_role: mapping.litellm_user_role,
                })
                .collect(),
            user_header_name: input.user_header_name.clone(),
            add_user_information: false,
            forward_openai_org_id: false,
            model_group_forwarding: None,
            forward_client_headers: false,
            forward_traceparent: false,
        };
        let settings = IdSettings {
            request_headers: &request_headers,
            validate_end_user_id_in_db: input.validate_end_user_id_in_db,
        };
        let mut ids = Vec::with_capacity(input.requests.len());
        for request in &input.requests {
            let headers = request.headers.as_deref().map(header_map).transpose()?;
            ids.push(request_end_user_id(
                &request.body,
                headers.as_ref(),
                &settings,
            ));
        }
        Ok(json!({ "ids": ids }))
    }
}

// Starlette decodes header bytes as latin-1, so the case's text maps back one character per byte
fn header_map(pairs: &[(String, String)]) -> Result<HeaderMap, Box<HelperError>> {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        let header_name = HeaderName::from_bytes(name.as_bytes()).or_value_error()?;
        let bytes = value
            .chars()
            .map(|character| u8::try_from(u32::from(character)))
            .collect::<Result<Vec<u8>, _>>()
            .or_value_error()?;
        headers.append(
            header_name,
            HeaderValue::from_bytes(&bytes).or_value_error()?,
        );
    }
    Ok(headers)
}
