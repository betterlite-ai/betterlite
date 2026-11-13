use betterlite_http::url_path::encode_segment;
use secrecy::SecretString;
use serde_json::Value;

use crate::evals::decode::decode_eval_reply;
use crate::evals::error::EvalsError;
use crate::evals::transformation::{BaseEvalsAPIConfig, EvalsHttpResponse, EvalsRequestContext};
use crate::evals::types::{
    CancelEvalResponse, CancelRunResponse, CreateEvalRequest, CreateRunRequest, DeleteEvalResponse,
    Eval, ListEvalsParams, ListEvalsResponse, ListRunsParams, ListRunsResponse, Run,
    RunDeleteResponse, UpdateEvalRequest,
};
use crate::openai_base_config::{Headers, Object};
use fields::{context_api_base, evals_query, present, present_secret, request_object, runs_query};

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
                let encoded = encode_segment(eval_id, "eval_id")
                    .map_err(|source| EvalsError::PathSegment { source })?;
                Ok(format!("{api_base}/v1/evals/{encoded}"))
            }
            None => Ok(format!("{api_base}/v1/{endpoint}")),
        }
    }

    fn transform_create_eval_request(
        &self,
        create_request: &CreateEvalRequest,
        _context: &EvalsRequestContext<'_>,
    ) -> Result<Object, Self::Error> {
        request_object(create_request)
    }

    fn transform_create_eval_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<Eval, Self::Error> {
        let reply = decode_eval_reply(raw_response.text.as_bytes())?;
        serde_json::from_value(reply).map_err(|source| EvalsError::ReplyModel { source })
    }

    fn transform_list_evals_request(
        &self,
        list_params: &ListEvalsParams,
        context: &EvalsRequestContext<'_>,
    ) -> Result<(String, Object), Self::Error> {
        let api_base = context_api_base(context);
        let url = self.get_complete_url(Some(api_base), "evals", None)?;
        Ok((url, evals_query(list_params)))
    }

    fn transform_list_evals_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<ListEvalsResponse, Self::Error> {
        let reply = decode_eval_reply(raw_response.text.as_bytes())?;
        serde_json::from_value(reply).map_err(|source| EvalsError::ReplyModel { source })
    }

    fn transform_get_eval_request(
        &self,
        eval_id: &str,
        api_base: &str,
        context: &EvalsRequestContext<'_>,
    ) -> Result<(String, Headers), Self::Error> {
        let url = self.get_complete_url(Some(api_base), "evals", Some(eval_id))?;
        Ok((url, context.headers.clone()))
    }

    fn transform_get_eval_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<Eval, Self::Error> {
        let reply = decode_eval_reply(raw_response.text.as_bytes())?;
        serde_json::from_value(reply).map_err(|source| EvalsError::ReplyModel { source })
    }

    fn transform_update_eval_request(
        &self,
        eval_id: &str,
        update_request: &UpdateEvalRequest,
        api_base: &str,
        context: &EvalsRequestContext<'_>,
    ) -> Result<(String, Headers, Object), Self::Error> {
        let url = self.get_complete_url(Some(api_base), "evals", Some(eval_id))?;
        Ok((
            url,
            context.headers.clone(),
            request_object(update_request)?,
        ))
    }

    fn transform_update_eval_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<Eval, Self::Error> {
        let reply = decode_eval_reply(raw_response.text.as_bytes())?;
        serde_json::from_value(reply).map_err(|source| EvalsError::ReplyModel { source })
    }

    fn transform_delete_eval_request(
        &self,
        eval_id: &str,
        api_base: &str,
        context: &EvalsRequestContext<'_>,
    ) -> Result<(String, Headers), Self::Error> {
        let url = self.get_complete_url(Some(api_base), "evals", Some(eval_id))?;
        Ok((url, context.headers.clone()))
    }

    fn transform_delete_eval_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<DeleteEvalResponse, Self::Error> {
        let reply = decode_eval_reply(raw_response.text.as_bytes())?;
        serde_json::from_value(reply).map_err(|source| EvalsError::ReplyModel { source })
    }

    fn transform_cancel_eval_request(
        &self,
        eval_id: &str,
        api_base: &str,
        context: &EvalsRequestContext<'_>,
    ) -> Result<(String, Headers, Object), Self::Error> {
        let base = self.get_complete_url(Some(api_base), "evals", Some(eval_id))?;
        Ok((
            format!("{base}/cancel"),
            context.headers.clone(),
            Object::new(),
        ))
    }

    fn transform_cancel_eval_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<CancelEvalResponse, Self::Error> {
        let reply = decode_eval_reply(raw_response.text.as_bytes())?;
        serde_json::from_value(reply).map_err(|source| EvalsError::ReplyModel { source })
    }

    fn transform_create_run_request(
        &self,
        eval_id: &str,
        create_request: &CreateRunRequest,
        context: &EvalsRequestContext<'_>,
    ) -> Result<(String, Object), Self::Error> {
        let api_base = context_api_base(context);
        let encoded = encode_segment(eval_id, "eval_id")
            .map_err(|source| EvalsError::PathSegment { source })?;
        Ok((
            format!("{api_base}/v1/evals/{encoded}/runs"),
            request_object(create_request)?,
        ))
    }

    fn transform_create_run_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<Run, Self::Error> {
        let reply = decode_eval_reply(raw_response.text.as_bytes())?;
        serde_json::from_value(reply).map_err(|source| EvalsError::ReplyModel { source })
    }

    fn transform_list_runs_request(
        &self,
        eval_id: &str,
        list_params: &ListRunsParams,
        context: &EvalsRequestContext<'_>,
    ) -> Result<(String, Object), Self::Error> {
        let api_base = context_api_base(context);
        let encoded = encode_segment(eval_id, "eval_id")
            .map_err(|source| EvalsError::PathSegment { source })?;
        Ok((
            format!("{api_base}/v1/evals/{encoded}/runs"),
            runs_query(list_params),
        ))
    }

    fn transform_list_runs_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<ListRunsResponse, Self::Error> {
        let reply = decode_eval_reply(raw_response.text.as_bytes())?;
        serde_json::from_value(reply).map_err(|source| EvalsError::ReplyModel { source })
    }

    fn transform_get_run_request(
        &self,
        eval_id: &str,
        run_id: &str,
        api_base: &str,
        context: &EvalsRequestContext<'_>,
    ) -> Result<(String, Headers), Self::Error> {
        let encoded_eval = encode_segment(eval_id, "eval_id")
            .map_err(|source| EvalsError::PathSegment { source })?;
        let encoded_run = encode_segment(run_id, "run_id")
            .map_err(|source| EvalsError::PathSegment { source })?;
        Ok((
            format!("{api_base}/v1/evals/{encoded_eval}/runs/{encoded_run}"),
            context.headers.clone(),
        ))
    }

    fn transform_get_run_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<Run, Self::Error> {
        let reply = decode_eval_reply(raw_response.text.as_bytes())?;
        serde_json::from_value(reply).map_err(|source| EvalsError::ReplyModel { source })
    }

    fn transform_cancel_run_request(
        &self,
        eval_id: &str,
        run_id: &str,
        api_base: &str,
        context: &EvalsRequestContext<'_>,
    ) -> Result<(String, Headers, Object), Self::Error> {
        let encoded_eval = encode_segment(eval_id, "eval_id")
            .map_err(|source| EvalsError::PathSegment { source })?;
        let encoded_run = encode_segment(run_id, "run_id")
            .map_err(|source| EvalsError::PathSegment { source })?;
        Ok((
            format!("{api_base}/v1/evals/{encoded_eval}/runs/{encoded_run}/cancel"),
            context.headers.clone(),
            Object::new(),
        ))
    }

    fn transform_cancel_run_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<CancelRunResponse, Self::Error> {
        let reply = decode_eval_reply(raw_response.text.as_bytes())?;
        serde_json::from_value(reply).map_err(|source| EvalsError::ReplyModel { source })
    }

    fn transform_delete_run_request(
        &self,
        eval_id: &str,
        run_id: &str,
        api_base: &str,
        context: &EvalsRequestContext<'_>,
    ) -> Result<(String, Headers, Object), Self::Error> {
        let encoded_eval = encode_segment(eval_id, "eval_id")
            .map_err(|source| EvalsError::PathSegment { source })?;
        let encoded_run = encode_segment(run_id, "run_id")
            .map_err(|source| EvalsError::PathSegment { source })?;
        Ok((
            format!("{api_base}/v1/evals/{encoded_eval}/runs/{encoded_run}"),
            context.headers.clone(),
            Object::new(),
        ))
    }

    fn transform_delete_run_response(
        &self,
        raw_response: &EvalsHttpResponse<'_>,
    ) -> Result<RunDeleteResponse, Self::Error> {
        let reply = decode_eval_reply(raw_response.text.as_bytes())?;
        serde_json::from_value(reply).map_err(|source| EvalsError::ReplyModel { source })
    }
}
