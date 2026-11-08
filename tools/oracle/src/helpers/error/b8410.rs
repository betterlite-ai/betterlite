use std::collections::BTreeMap;

use betterlite_error::proxy_enums::special_headers::SpecialHeaders;
// ast-grep-ignore: no-long-names
use betterlite_error::proxy_exception::model_access_denied::ModelAccessDeniedProxyException;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use serde::Deserialize;
use serde_json::{Map, Value};

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Input {
    Exception {
        message: String,
        #[serde(rename = "type")]
        error_type: String,
        param: Option<String>,
        code: Option<Code>,
        #[serde(default)]
        openai_code: Option<String>,
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default)]
        provider_specific_fields: Option<Map<String, Value>>,
    },
    AccessDenied {
        message: String,
        internal_message: String,
        #[serde(rename = "type")]
        error_type: String,
        param: Option<String>,
        code: Option<Code>,
    },
    CredentialHeaders,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Code {
    Status(i64),
    Text(String),
}

fn exception_code(code: Option<&Code>) -> ExceptionCode {
    match code {
        Some(Code::Status(status)) => ExceptionCode::Status(*status),
        Some(Code::Text(text)) => ExceptionCode::Text(text.clone()),
        None => ExceptionCode::Absent,
    }
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let output = match input {
            Input::Exception {
                message,
                error_type,
                param,
                code,
                openai_code,
                headers,
                provider_specific_fields,
            } => {
                let exception = ProxyException::new(ProxyExceptionInit {
                    message: message.clone(),
                    error_type: error_type.clone(),
                    param: param.clone(),
                    code: exception_code(code.as_ref()),
                    headers: headers.clone(),
                    openai_code: openai_code.clone(),
                    provider_specific_fields: provider_specific_fields.clone(),
                });
                serde_json::json!({
                    "code": exception.code,
                    "openai_code": exception.openai_code,
                    "body": exception.to_body(),
                    "headers": exception.headers,
                })
            }
            Input::AccessDenied {
                message,
                internal_message,
                error_type,
                param,
                code,
            } => {
                // ast-grep-ignore: no-long-names
                let denied = ModelAccessDeniedProxyException::new(
                    message.clone(),
                    internal_message.clone(),
                    error_type.clone(),
                    param.clone(),
                    exception_code(code.as_ref()),
                );
                serde_json::json!({
                    "sanitized": denied.sanitized_internal_message(),
                    "body": denied.as_proxy_exception().to_body(),
                })
            }
            Input::CredentialHeaders => {
                // ast-grep-ignore: no-long-names
                let names = SpecialHeaders::litellm_credential_header_names();
                serde_json::json!({ "names": names })
            }
        };
        Ok(output)
    }
}
