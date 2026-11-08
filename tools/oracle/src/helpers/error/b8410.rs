use std::collections::BTreeMap;

use betterlite_error::proxy_enums::special_headers::SpecialHeaders;
// ast-grep-ignore: no-long-names
use betterlite_error::proxy_exception::model_access_denied::ModelAccessDeniedProxyException;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_redact::structured::Payload;
use serde::Deserialize;
use serde_json::Value;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
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
        provider_specific_fields: BTreeMap<String, Payload>,
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
pub(crate) enum Code {
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

fn exception_output(init: ProxyExceptionInit) -> Value {
    let exception = ProxyException::new(init);
    serde_json::json!({
        "code": exception.code,
        "openai_code": exception.openai_code,
        "body": exception.to_body(),
        "headers": exception.headers,
    })
}

fn access_denied_output(
    message: &str,
    internal_message: &str,
    error_type: &str,
    param: Option<&str>,
    code: Option<&Code>,
) -> Value {
    // ast-grep-ignore: no-long-names
    let denied = ModelAccessDeniedProxyException::new(
        message.to_owned(),
        internal_message.to_owned(),
        error_type.to_owned(),
        param.map(str::to_owned),
        exception_code(code),
    );
    serde_json::json!({
        "sanitized": denied.sanitized_internal_message(),
        "body": denied.as_proxy_exception().to_body(),
    })
}

fn credential_headers_output() -> Value {
    // ast-grep-ignore: no-long-names
    let names = SpecialHeaders::litellm_credential_header_names();
    serde_json::json!({ "names": names })
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
            } => exception_output(ProxyExceptionInit {
                message: message.clone(),
                error_type: error_type.clone(),
                param: param.clone(),
                code: exception_code(code.as_ref()),
                headers: headers.clone(),
                openai_code: openai_code.clone(),
                provider_specific_fields: provider_specific_fields.clone(),
            }),
            Input::AccessDenied {
                message,
                internal_message,
                error_type,
                param,
                code,
            } => access_denied_output(
                message,
                internal_message,
                error_type,
                param.as_deref(),
                code.as_ref(),
            ),
            Input::CredentialHeaders => credential_headers_output(),
        };
        Ok(output)
    }
}
