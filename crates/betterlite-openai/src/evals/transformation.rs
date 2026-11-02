use crate::error::{ProviderError, ProviderErrorParts};
use crate::evals::types::{
    CancelEvalResponse, CancelRunResponse, CreateEvalRequest, CreateRunRequest, DeleteEvalResponse,
    Eval, ListEvalsParams, ListEvalsResponse, ListRunsParams, ListRunsResponse, Run,
    RunDeleteResponse, UpdateEvalRequest,
};
use crate::openai_base_config::{Headers, Object};

mod tests;

#[derive(Debug, Clone, Copy)]
pub struct EvalsHttpResponse<'call> {
    pub status_code: u16,
    pub headers: &'call Headers,
    pub text: &'call str,
}

#[derive(Debug, Clone, Copy)]
pub struct EvalsRequestContext<'call> {
    pub litellm_params: &'call Object,
    pub headers: &'call Headers,
}

pub trait BaseEvalsAPIConfig {
    type Error: From<ProviderError>;

    fn validate_environment(
        &self,
        headers: Headers,
        litellm_params: Option<&Object>,
    ) -> Result<Headers, Self::Error>;

    fn get_complete_url(
        &self,
        api_base: Option<&str>,
        endpoint: &str,
        eval_id: Option<&str>,
    ) -> Result<String, Self::Error>;

    fn transform_create_eval_request(
        &self,
        create_request: &CreateEvalRequest,
        context: &EvalsRequestContext<'_>,
    ) -> Result<Object, Self::Error>;

    fn transform_create_eval_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<Eval, Self::Error>;

    fn transform_list_evals_request(
        &self,
        list_params: &ListEvalsParams,
        context: &EvalsRequestContext<'_>,
    ) -> Result<(String, Object), Self::Error>;

    fn transform_list_evals_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<ListEvalsResponse, Self::Error>;

    fn transform_get_eval_request(
        &self,
        eval_id: &str,
        api_base: &str,
        context: &EvalsRequestContext<'_>,
    ) -> Result<(String, Headers), Self::Error>;

    fn transform_get_eval_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<Eval, Self::Error>;

    fn transform_update_eval_request(
        &self,
        eval_id: &str,
        update_request: &UpdateEvalRequest,
        api_base: &str,
        context: &EvalsRequestContext<'_>,
    ) -> Result<(String, Headers, Object), Self::Error>;

    fn transform_update_eval_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<Eval, Self::Error>;

    fn transform_delete_eval_request(
        &self,
        eval_id: &str,
        api_base: &str,
        context: &EvalsRequestContext<'_>,
    ) -> Result<(String, Headers), Self::Error>;

    fn transform_delete_eval_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<DeleteEvalResponse, Self::Error>;

    fn transform_cancel_eval_request(
        &self,
        eval_id: &str,
        api_base: &str,
        context: &EvalsRequestContext<'_>,
    ) -> Result<(String, Headers, Object), Self::Error>;

    fn transform_cancel_eval_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<CancelEvalResponse, Self::Error>;

    fn transform_create_run_request(
        &self,
        eval_id: &str,
        create_request: &CreateRunRequest,
        context: &EvalsRequestContext<'_>,
    ) -> Result<(String, Object), Self::Error>;

    fn transform_create_run_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<Run, Self::Error>;

    fn transform_list_runs_request(
        &self,
        eval_id: &str,
        list_params: &ListRunsParams,
        context: &EvalsRequestContext<'_>,
    ) -> Result<(String, Object), Self::Error>;

    fn transform_list_runs_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<ListRunsResponse, Self::Error>;

    fn transform_get_run_request(
        &self,
        eval_id: &str,
        run_id: &str,
        api_base: &str,
        context: &EvalsRequestContext<'_>,
    ) -> Result<(String, Headers), Self::Error>;

    fn transform_get_run_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<Run, Self::Error>;

    fn transform_cancel_run_request(
        &self,
        eval_id: &str,
        run_id: &str,
        api_base: &str,
        context: &EvalsRequestContext<'_>,
    ) -> Result<(String, Headers, Object), Self::Error>;

    fn transform_cancel_run_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<CancelRunResponse, Self::Error>;

    fn transform_delete_run_request(
        &self,
        eval_id: &str,
        run_id: &str,
        api_base: &str,
        context: &EvalsRequestContext<'_>,
    ) -> Result<(String, Headers, Object), Self::Error>;

    fn transform_delete_run_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<RunDeleteResponse, Self::Error>;

    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &Headers,
    ) -> Self::Error {
        Self::Error::from(ProviderError::from(ProviderErrorParts {
            status_code,
            message: error_message.to_owned(),
            headers: Some(headers.clone().into_iter().collect()),
            request: None,
            response: None,
            body: None,
            status_code_is_synthesized: false,
        }))
    }
}
