pub mod error;
pub mod params;
pub mod reply;
pub mod request;

mod tests;

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::{
    RerankResponse, RerankResponseDocument, RerankResponseMeta, RerankResponseResult, RerankTokens,
};
use betterlite_openai::watsonx_auth::WatsonxAuth;
use betterlite_openai::watsonx_auth::error::WatsonxError;
use betterlite_openai::watsonx_auth::transport::WatsonxTransport;
use betterlite_openai::watsonx_auth::types::WatsonxEndpoint;
use betterlite_openai::watsonx_auth::urls::{add_api_version_to_url, get_base_url};
use secrecy::{ExposeSecret as _, SecretString};
use uuid::Uuid;

use self::error::WatsonxRerankError;
use self::reply::{WatsonxRerankResult, decode_watsonx_rerank_reply};
use crate::api::rerank_contracts::{Headers, Params, RerankProviderResponse};

const AUTHORIZATION: &str = "Authorization";
const CONTENT_TYPE: &str = "Content-Type";
const ACCEPT: &str = "Accept";

pub struct WatsonxRerankConfig<'env, T> {
    pub auth: &'env WatsonxAuth<T>,
    pub new_id: fn() -> Uuid,
}

impl<T: WatsonxTransport> WatsonxRerankConfig<'_, T> {
    // the rerank config pops token and zen_api_key where the shared getter only reads the token
    pub async fn validate_environment(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
        options: &mut Params,
    ) -> Result<Headers, WatsonxRerankError> {
        let mut resolved = Headers::new();
        resolved.insert(String::from(CONTENT_TYPE), String::from("application/json"));
        resolved.insert(String::from(ACCEPT), String::from("application/json"));
        if headers.contains_key(AUTHORIZATION) {
            resolved.extend(headers);
            return Ok(resolved);
        }
        let token = take_secret(options, "token")?
            .or_else(|| non_empty(self.auth.environment.watsonx_token.clone()));
        let zen_api_key = take_secret(options, "zen_api_key")?
            .or_else(|| non_empty(self.auth.environment.watsonx_zenapikey.clone()));
        let authorization = match (token, zen_api_key) {
            (Some(token), _) => format!("Bearer {}", token.expose_secret()),
            (None, Some(zen_api_key)) => format!("ZenApiKey {}", zen_api_key.expose_secret()),
            (None, None) => {
                let generated = self.auth.generate_iam_token(api_key).await?;
                format!("Bearer {}", generated.expose_secret())
            }
        };
        resolved.extend(headers);
        resolved.insert(String::from(AUTHORIZATION), authorization);
        Ok(resolved)
    }

    pub fn get_complete_url(
        &self,
        api_base: Option<&str>,
        options: Option<&Params>,
    ) -> Result<String, WatsonxRerankError> {
        let base = get_base_url(api_base, &self.auth.environment)?;
        let url = format!(
            "{}{}",
            base.trim_end_matches('/'),
            WatsonxEndpoint::Rerank.path()
        );
        let version = match options.and_then(|params| params.get("api_version")) {
            None | Some(Json::Null) => None,
            Some(Json::Text(version)) => Some(version.as_str()),
            Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
                return Err(WatsonxRerankError::NotText {
                    field: "api_version",
                });
            }
        };
        Ok(add_api_version_to_url(&url, version))
    }

    pub fn transform_rerank_response(
        &self,
        raw: &RerankProviderResponse,
    ) -> Result<RerankResponse, WatsonxRerankError> {
        let reply = match decode_watsonx_rerank_reply(&raw.body) {
            Ok(reply) => reply,
            Err(WatsonxRerankError::Parse(source)) => {
                return Err(WatsonxRerankError::Watsonx(Box::new(WatsonxError::Api {
                    status_code: raw.status_code,
                    message: format!("Failed to parse response: {source}"),
                    headers: None,
                })));
            }
            Err(other) => return Err(other),
        };
        let Some(results) = reply.results else {
            return Err(WatsonxRerankError::NoResults {
                body: body_text(&raw.body),
            });
        };
        let mut transformed = Vec::with_capacity(results.len());
        for result in results {
            transformed.push(transform_result(result)?);
        }
        let id = match reply.id {
            Some(id) if !id.is_empty() => id,
            Some(_) | None => (self.new_id)().to_string(),
        };
        let input_tokens = reply.input_token_count.unwrap_or(Some(0_i64));
        Ok(RerankResponse {
            id: Some(id),
            results: Some(transformed),
            meta: Some(RerankResponseMeta {
                api_version: None,
                billed_units: None,
                tokens: Some(Some(RerankTokens {
                    input_tokens: Some(input_tokens),
                    output_tokens: None,
                })),
            }),
            hidden_params: BTreeMap::new(),
        })
    }
}

fn transform_result(
    result: WatsonxRerankResult,
) -> Result<RerankResponseResult, WatsonxRerankError> {
    let document = match result.input {
        None => None,
        Some(Some(Json::Text(text))) => Some(RerankResponseDocument { text }),
        Some(Some(Json::Object(fields))) => match fields.get("text") {
            Some(Json::Text(text)) => Some(RerankResponseDocument { text: text.clone() }),
            _ => {
                return Err(WatsonxRerankError::ReplyField {
                    field: "results[].input.text",
                });
            }
        },
        Some(None) | Some(Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_))) => {
            return Err(WatsonxRerankError::ReplyField {
                field: "results[].input",
            });
        }
    };
    Ok(RerankResponseResult {
        index: result.index,
        relevance_score: result.score,
        document,
    })
}

fn take_secret(
    options: &mut Params,
    field: &'static str,
) -> Result<Option<SecretString>, WatsonxRerankError> {
    match options.remove(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) if text.is_empty() => Ok(None),
        Some(Json::Text(text)) => Ok(Some(SecretString::from(text))),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(WatsonxRerankError::NotText { field })
        }
    }
}

fn non_empty(secret: Option<SecretString>) -> Option<SecretString> {
    secret.filter(|secret| !secret.expose_secret().is_empty())
}

fn body_text(bytes: &[u8]) -> String {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return format!("{} bytes that are not UTF-8", bytes.len());
    };
    text.to_owned()
}
