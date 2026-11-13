use betterlite_http::url_path::encode_segment;
use secrecy::SecretString;
use serde_json::Value;

use crate::evals::error::EvalsError;
use crate::evals::transformation::{BaseEvalsAPIConfig, EvalsHttpResponse, EvalsRequestContext};
use crate::evals::types::{
    CancelEvalResponse, CancelRunResponse, CreateEvalRequest, CreateRunRequest, DeleteEvalResponse,
    Eval, ListEvalsParams, ListEvalsResponse, ListRunsParams, ListRunsResponse, Run,
    RunDeleteResponse, UpdateEvalRequest,
};
use crate::openai_base_config::{Headers, Object};
use fields::{
    context_api_base, evals_query, parse_reply, present, present_secret, request_object, runs_query,
};

mod fields;

#[derive(Debug, Default)]
pub struct OpenAiEvalsConfig {
    pub api_key: Option<SecretString>,
    pub openai_key: Option<SecretString>,
    pub env_api_key: Option<SecretString>,
}

impl BaseEvalsAPIConfig for OpenAiEvalsConfig {
    type Error = EvalsError;

    fn validate_environment(
        &self,
        mut headers: Headers,
        litellm_params: Option<&Object>,
    ) -> Result<Headers, Self::Error> {
        let explicit = litellm_params
            .and_then(|params| params.get("api_key"))
            .and_then(Value::as_str);
        let api_key = present(explicit)
            .or_else(|| present_secret(self.api_key.as_ref()))
            .or_else(|| present_secret(self.openai_key.as_ref()))
            .or_else(|| present_secret(self.env_api_key.as_ref()))
            .ok_or(EvalsError::ApiKeyRequired)?;
        headers.insert("Authorization".to_owned(), format!("Bearer {api_key}"));
        headers.insert("Content-Type".to_owned(), "application/json".to_owned());
        Ok(headers)
    }

    fn get_complete_url(
        &self,
        api_base: Option<&str>,
        endpoint: &str,
        eval_id: Option<&str>,
    ) -> Result<String, Self::Error> {
        let api_base = api_base.unwrap_or(fields::EVALS_API_BASE);
        match eval_id.filter(|eval_id| !eval_id.is_empty()) {
            Some(eval_id) => {
                let encoded = encode_segment(eval_id, "eval_id")?;
                Ok(format!("{api_base}/v1/evals/{encoded}"))
            }
            None => Ok(format!("{api_base}/v1/{endpoint}")),
        }
    }

    #[rustfmt::skip]
    fn transform_create_eval_request(&self, create_request: &CreateEvalRequest, _context: &EvalsRequestContext<'_>) -> Result<Object, Self::Error> {
        request_object(create_request)
    }

    #[rustfmt::skip]
    fn transform_create_eval_response(&self, raw_response: &EvalsHttpResponse<'_>) -> Result<Eval, Self::Error> {
        parse_reply(raw_response)
    }

    #[rustfmt::skip]
    fn transform_list_evals_request(&self, list_params: &ListEvalsParams, context: &EvalsRequestContext<'_>) -> Result<(String, Object), Self::Error> {
        let url = self.get_complete_url(Some(context_api_base(context)), "evals", None)?;
        Ok((url, evals_query(list_params)))
    }

    #[rustfmt::skip]
    fn transform_list_evals_response(&self, raw_response: &EvalsHttpResponse<'_>) -> Result<ListEvalsResponse, Self::Error> {
        parse_reply(raw_response)
    }

    #[rustfmt::skip]
    fn transform_get_eval_request(&self, eval_id: &str, api_base: &str, context: &EvalsRequestContext<'_>) -> Result<(String, Headers), Self::Error> {
        Ok((self.get_complete_url(Some(api_base), "evals", Some(eval_id))?, context.headers.clone()))
    }

    #[rustfmt::skip]
    fn transform_get_eval_response(&self, raw_response: &EvalsHttpResponse<'_>) -> Result<Eval, Self::Error> {
        parse_reply(raw_response)
    }

    #[rustfmt::skip]
    fn transform_update_eval_request(&self, eval_id: &str, update_request: &UpdateEvalRequest, api_base: &str, context: &EvalsRequestContext<'_>) -> Result<(String, Headers, Object), Self::Error> {
        let url = self.get_complete_url(Some(api_base), "evals", Some(eval_id))?;
        Ok((url, context.headers.clone(), request_object(update_request)?))
    }

    #[rustfmt::skip]
    fn transform_update_eval_response(&self, raw_response: &EvalsHttpResponse<'_>) -> Result<Eval, Self::Error> {
        parse_reply(raw_response)
    }

    #[rustfmt::skip]
    fn transform_delete_eval_request(&self, eval_id: &str, api_base: &str, context: &EvalsRequestContext<'_>) -> Result<(String, Headers), Self::Error> {
        Ok((self.get_complete_url(Some(api_base), "evals", Some(eval_id))?, context.headers.clone()))
    }

    #[rustfmt::skip]
    fn transform_delete_eval_response(&self, raw_response: &EvalsHttpResponse<'_>) -> Result<DeleteEvalResponse, Self::Error> {
        parse_reply(raw_response)
    }

    #[rustfmt::skip]
    fn transform_cancel_eval_request(&self, eval_id: &str, api_base: &str, context: &EvalsRequestContext<'_>) -> Result<(String, Headers, Object), Self::Error> {
        let base = self.get_complete_url(Some(api_base), "evals", Some(eval_id))?;
        Ok((format!("{base}/cancel"), context.headers.clone(), Object::new()))
    }

    #[rustfmt::skip]
    fn transform_cancel_eval_response(&self, raw_response: &EvalsHttpResponse<'_>) -> Result<CancelEvalResponse, Self::Error> {
        parse_reply(raw_response)
    }

    #[rustfmt::skip]
    fn transform_create_run_request(&self, eval_id: &str, create_request: &CreateRunRequest, context: &EvalsRequestContext<'_>) -> Result<(String, Object), Self::Error> {
        let encoded = encode_segment(eval_id, "eval_id")?;
        Ok((format!("{}/v1/evals/{encoded}/runs", context_api_base(context)), request_object(create_request)?))
    }

    #[rustfmt::skip]
    fn transform_create_run_response(&self, raw_response: &EvalsHttpResponse<'_>) -> Result<Run, Self::Error> {
        parse_reply(raw_response)
    }

    #[rustfmt::skip]
    fn transform_list_runs_request(&self, eval_id: &str, list_params: &ListRunsParams, context: &EvalsRequestContext<'_>) -> Result<(String, Object), Self::Error> {
        let encoded = encode_segment(eval_id, "eval_id")?;
        Ok((format!("{}/v1/evals/{encoded}/runs", context_api_base(context)), runs_query(list_params)))
    }

    #[rustfmt::skip]
    fn transform_list_runs_response(&self, raw_response: &EvalsHttpResponse<'_>) -> Result<ListRunsResponse, Self::Error> {
        parse_reply(raw_response)
    }

    #[rustfmt::skip]
    fn transform_get_run_request(&self, eval_id: &str, run_id: &str, api_base: &str, context: &EvalsRequestContext<'_>) -> Result<(String, Headers), Self::Error> {
        let eval_segment = encode_segment(eval_id, "eval_id")?;
        let run_segment = encode_segment(run_id, "run_id")?;
        Ok((format!("{api_base}/v1/evals/{eval_segment}/runs/{run_segment}"), context.headers.clone()))
    }

    #[rustfmt::skip]
    fn transform_get_run_response(&self, raw_response: &EvalsHttpResponse<'_>) -> Result<Run, Self::Error> {
        parse_reply(raw_response)
    }

    #[rustfmt::skip]
    fn transform_cancel_run_request(&self, eval_id: &str, run_id: &str, api_base: &str, context: &EvalsRequestContext<'_>) -> Result<(String, Headers, Object), Self::Error> {
        let eval_segment = encode_segment(eval_id, "eval_id")?;
        let run_segment = encode_segment(run_id, "run_id")?;
        Ok((format!("{api_base}/v1/evals/{eval_segment}/runs/{run_segment}/cancel"), context.headers.clone(), Object::new()))
    }

    #[rustfmt::skip]
    fn transform_cancel_run_response(&self, raw_response: &EvalsHttpResponse<'_>) -> Result<CancelRunResponse, Self::Error> {
        parse_reply(raw_response)
    }

    #[rustfmt::skip]
    fn transform_delete_run_request(&self, eval_id: &str, run_id: &str, api_base: &str, context: &EvalsRequestContext<'_>) -> Result<(String, Headers, Object), Self::Error> {
        let eval_segment = encode_segment(eval_id, "eval_id")?;
        let run_segment = encode_segment(run_id, "run_id")?;
        Ok((format!("{api_base}/v1/evals/{eval_segment}/runs/{run_segment}"), context.headers.clone(), Object::new()))
    }

    #[rustfmt::skip]
    fn transform_delete_run_response(&self, raw_response: &EvalsHttpResponse<'_>) -> Result<RunDeleteResponse, Self::Error> {
        parse_reply(raw_response)
    }
}
