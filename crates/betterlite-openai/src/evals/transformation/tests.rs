#![cfg(test)]

use serde::de::DeserializeOwned;

use crate::error::{ProviderError, ProviderErrorParts};
use crate::evals::transformation::{BaseEvalsAPIConfig, EvalsHttpResponse, EvalsRequestContext};
use crate::evals::types::{
    CancelEvalResponse, CancelRunResponse, CancelledStatus, CreateEvalRequest, CreateRunRequest,
    DeleteEvalResponse, Eval, ListEvalsParams, ListEvalsResponse, ListRunsParams, ListRunsResponse,
    Run, RunDeleteResponse, RunStatus, UpdateEvalRequest,
};
use crate::openai_base_config::{Headers, Object};

macro_rules! object_echo_request {
    ($($name:ident($($arg:ident: $ty:ty),*);)+) => {
        $(
            fn $name(
                &self,
                $($arg: $ty,)*
                _context: &EvalsRequestContext<'_>,
            ) -> Result<(String, Object), Self::Error> {
                Ok(echo_object())
            }
        )+
    };
}

macro_rules! headers_echo_request {
    ($($name:ident($($arg:ident: $ty:ty),*);)+) => {
        $(
            fn $name(
                &self,
                $($arg: $ty,)*
                context: &EvalsRequestContext<'_>,
            ) -> Result<(String, Headers), Self::Error> {
                Ok(echo_headers(context))
            }
        )+
    };
}

macro_rules! headers_object_echo_request {
    ($($name:ident($($arg:ident: $ty:ty),*);)+) => {
        $(
            fn $name(
                &self,
                $($arg: $ty,)*
                context: &EvalsRequestContext<'_>,
            ) -> Result<(String, Headers, Object), Self::Error> {
                Ok(echo_headers_and_object(context))
            }
        )+
    };
}

struct NoOpEvalsConfig;

impl BaseEvalsAPIConfig for NoOpEvalsConfig {
    type Error = ProviderError;

    fn validate_environment(
        &self,
        headers: Headers,
        _litellm_params: Option<&Object>,
    ) -> Result<Headers, Self::Error> {
        Ok(headers)
    }

    fn get_complete_url(
        &self,
        api_base: Option<&str>,
        endpoint: &str,
        _eval_id: Option<&str>,
    ) -> Result<String, Self::Error> {
        Ok(format!("{}/v1/{endpoint}", api_base.unwrap_or("")))
    }

    fn transform_create_eval_request(
        &self,
        _create_request: &CreateEvalRequest,
        _context: &EvalsRequestContext<'_>,
    ) -> Result<Object, Self::Error> {
        Ok(Object::new())
    }

    fn transform_create_eval_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<Eval, Self::Error> {
        parse_response(raw_response)
    }

    object_echo_request! {
        transform_list_evals_request(_list_params: &ListEvalsParams);
    }

    fn transform_list_evals_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<ListEvalsResponse, Self::Error> {
        parse_response(raw_response)
    }

    headers_echo_request! {
        transform_get_eval_request(_eval_id: &str, _api_base: &str);
    }

    fn transform_get_eval_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<Eval, Self::Error> {
        parse_response(raw_response)
    }

    headers_object_echo_request! {
        transform_update_eval_request(_eval_id: &str, _update_request: &UpdateEvalRequest, _api_base: &str);
    }

    fn transform_update_eval_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<Eval, Self::Error> {
        parse_response(raw_response)
    }

    headers_echo_request! {
        transform_delete_eval_request(_eval_id: &str, _api_base: &str);
    }

    fn transform_delete_eval_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<DeleteEvalResponse, Self::Error> {
        parse_response(raw_response)
    }

    headers_object_echo_request! {
        transform_cancel_eval_request(_eval_id: &str, _api_base: &str);
    }

    fn transform_cancel_eval_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<CancelEvalResponse, Self::Error> {
        parse_response(raw_response)
    }

    object_echo_request! {
        transform_create_run_request(_eval_id: &str, _create_request: &CreateRunRequest);
    }

    fn transform_create_run_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<Run, Self::Error> {
        parse_response(raw_response)
    }

    object_echo_request! {
        transform_list_runs_request(_eval_id: &str, _list_params: &ListRunsParams);
    }

    fn transform_list_runs_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<ListRunsResponse, Self::Error> {
        parse_response(raw_response)
    }

    headers_echo_request! {
        transform_get_run_request(_eval_id: &str, _run_id: &str, _api_base: &str);
    }

    fn transform_get_run_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<Run, Self::Error> {
        parse_response(raw_response)
    }

    headers_object_echo_request! {
        transform_cancel_run_request(_eval_id: &str, _run_id: &str, _api_base: &str);
    }

    fn transform_cancel_run_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<CancelRunResponse, Self::Error> {
        parse_response(raw_response)
    }

    headers_object_echo_request! {
        transform_delete_run_request(_eval_id: &str, _run_id: &str, _api_base: &str);
    }

    fn transform_delete_run_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<RunDeleteResponse, Self::Error> {
        parse_response(raw_response)
    }
}

fn echo_object() -> (String, Object) {
    (String::new(), Object::new())
}

fn echo_headers(context: &EvalsRequestContext<'_>) -> (String, Headers) {
    (String::new(), context.headers.clone())
}

fn echo_headers_and_object(context: &EvalsRequestContext<'_>) -> (String, Headers, Object) {
    (String::new(), context.headers.clone(), Object::new())
}

fn parse_response<T, E>(raw_response: &EvalsHttpResponse<'_>) -> Result<T, E>
where
    T: DeserializeOwned,
    E: From<ProviderError>,
{
    serde_json::from_str(raw_response.text).map_err(|source| E::from(parse_error(&source)))
}

fn parse_error(source: &serde_json::Error) -> ProviderError {
    ProviderError::from(ProviderErrorParts {
        status_code: 500,
        message: source.to_string(),
        headers: None,
        request: None,
        response: None,
        body: None,
        status_code_is_synthesized: true,
    })
}

#[test]
fn init_has_no_side_effects_and_url_building_stays_abstract() {
    let config = NoOpEvalsConfig;
    assert_eq!(
        config.get_complete_url(Some("https://api.openai.com"), "evals", None),
        Ok("https://api.openai.com/v1/evals".to_owned())
    );
    assert_eq!(
        config.validate_environment(Headers::new(), None),
        Ok(Headers::new())
    );
}

#[test]
fn transform_create_eval_hooks_round_trip_the_eval_response() {
    let config = NoOpEvalsConfig;
    let request = CreateEvalRequest {
        name: None,
        data_source_config: serde_json::from_str(r#"{"type": "logs"}"#).unwrap(),
        testing_criteria: Vec::new(),
        metadata: None,
    };
    let litellm_params = Object::new();
    let headers = Headers::new();
    let context = EvalsRequestContext {
        litellm_params: &litellm_params,
        headers: &headers,
    };
    let body = config
        .transform_create_eval_request(&request, &context)
        .unwrap();
    assert_eq!(body, Object::new());
    let text = r#"{"id": "eval-1", "created_at": 1, "data_source_config": {"type": "logs"}, "testing_criteria": []}"#;
    let response = EvalsHttpResponse {
        status_code: 200,
        headers: &Headers::new(),
        text,
    };
    let eval = config.transform_create_eval_response(&response).unwrap();
    assert_eq!(eval.id, "eval-1");
    assert_eq!(eval.object, "eval");
}

#[test]
fn transform_run_hooks_round_trip_the_run_response() {
    let config = NoOpEvalsConfig;
    let text = r#"{"id": "run-1", "created_at": 1, "status": "queued", "data_source": {"type": "logs"}, "eval_id": "eval-1"}"#;
    let response = EvalsHttpResponse {
        status_code: 200,
        headers: &Headers::new(),
        text,
    };
    let run = config.transform_get_run_response(&response).unwrap();
    assert_eq!(run.id, "run-1");
    assert_eq!(run.status, RunStatus::Queued);
}

#[test]
fn transform_cancel_hooks_report_the_cancelled_status() {
    let config = NoOpEvalsConfig;
    let eval_text = r#"{"id": "eval-1", "status": "cancelled"}"#;
    let eval_response = EvalsHttpResponse {
        status_code: 200,
        headers: &Headers::new(),
        text: eval_text,
    };
    let cancelled_eval = config
        .transform_cancel_eval_response(&eval_response)
        .unwrap();
    assert_eq!(cancelled_eval.status, CancelledStatus::Cancelled);
    let run_text = r#"{"id": "run-1", "status": "cancelled"}"#;
    let run_response = EvalsHttpResponse {
        status_code: 200,
        headers: &Headers::new(),
        text: run_text,
    };
    let cancelled_run = config.transform_cancel_run_response(&run_response).unwrap();
    assert_eq!(cancelled_run.status, CancelledStatus::Cancelled);
}

#[test]
fn get_error_class_builds_provider_error_with_status_message_and_headers() {
    let config = NoOpEvalsConfig;
    let headers = Headers::from([("retry-after".to_owned(), "3".to_owned())]);
    let error: ProviderError = config.get_error_class("slow down", 429, &headers);
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "slow down");
    assert_eq!(
        error.headers,
        Some(vec![("retry-after".to_owned(), "3".to_owned())])
    );
    assert!(!error.status_code_is_synthesized);
}
