use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use betterlite_auth::auth_model_budgets::request_views::{ModelRewrite, RewriteFailure};
use betterlite_core::api::request_body::{
    JsonObject, ParsedBody, ParsedJson, parse_json_bytes, rewrite_request_model,
};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject as PathParams;

pub(crate) type SharedViews = Arc<Mutex<Option<BodyViews>>>;

#[derive(Clone, Debug)]
pub(crate) struct BodyViews {
    pub(crate) request_data: Option<JsonObject>,
    pub(crate) parsed_body: Option<ParsedBody>,
    pub(crate) json_cache: Option<JsonObject>,
    pub(crate) serialized_body: Option<Vec<u8>>,
    pub(crate) path_params: Option<PathParams>,
}

impl BodyViews {
    pub(crate) fn new(body: Option<&Bytes>, path_params: Option<PathParams>) -> Self {
        let request_data = body.and_then(|bytes| match parse_json_bytes(bytes) {
            ParsedJson::Object(data) => Some(data),
            ParsedJson::NotObject | ParsedJson::Invalid => None,
        });
        Self {
            request_data,
            parsed_body: None,
            json_cache: None,
            serialized_body: None,
            path_params,
        }
    }
}

impl ModelRewrite for BodyViews {
    fn set_request_model(&mut self, model: &str) {
        if let Some(data) = self.request_data.as_mut() {
            *data = rewrite_request_model(data, None, model).request_data;
        }
    }

    fn refresh_parsed_body(&mut self) {
        self.parsed_body = self.request_data.clone().map(ParsedBody::from_client_body);
    }

    fn refresh_json_cache(&mut self) {
        self.json_cache.clone_from(&self.request_data);
    }

    fn refresh_serialized_body(&mut self) -> Result<(), RewriteFailure> {
        let Some(data) = self.request_data.as_ref() else {
            return Err(RewriteFailure);
        };
        self.serialized_body = Some(serde_json::to_vec(data).map_err(|_error| RewriteFailure)?);
        Ok(())
    }

    fn set_path_model(&mut self, model: &str) {
        if let Some(params) = self.path_params.as_mut()
            && params.contains_key("model")
        {
            params.insert("model".to_owned(), Json::Text(model.to_owned()));
        }
    }
}
