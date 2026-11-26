use axum::body::Bytes;
use betterlite_auth::auth_model_budgets::request_views::{ModelRewrite, RewriteFailure};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

// The bodies LiteLLM keeps apart are one parsed object here, so a view the request did not carry stays absent.
pub(crate) struct BodyViews {
    request_data: Option<JsonObject>,
    parsed_body: Option<JsonObject>,
    json_cache: Option<JsonObject>,
    pub(crate) serialized_body: Option<Vec<u8>>,
    path_params: Option<JsonObject>,
}

impl BodyViews {
    pub(crate) fn new(body: Option<&Bytes>, path_params: Option<JsonObject>) -> Self {
        Self {
            request_data: body.and_then(|bytes| serde_json::from_slice::<JsonObject>(bytes).ok()),
            parsed_body: None,
            json_cache: None,
            serialized_body: None,
            path_params,
        }
    }

    fn mirror(&mut self, target: fn(&mut Self) -> Option<&mut JsonObject>) {
        let Some(data) = self.request_data.clone() else {
            return;
        };
        if let Some(view) = target(self) {
            *view = data;
        }
    }
}

impl ModelRewrite for BodyViews {
    fn set_request_model(&mut self, model: &str) {
        if let Some(data) = self.request_data.as_mut() {
            data.insert("model".to_owned(), Json::Text(model.to_owned()));
        }
    }

    fn refresh_parsed_body(&mut self) {
        self.mirror(|views| views.parsed_body.as_mut());
    }

    fn refresh_json_cache(&mut self) {
        self.mirror(|views| views.json_cache.as_mut());
    }

    fn refresh_serialized_body(&mut self) -> Result<(), RewriteFailure> {
        let Some(data) = self.request_data.as_ref() else {
            return Err(RewriteFailure);
        };
        let body = serde_json::to_vec(data).map_err(|_error| RewriteFailure)?;
        self.serialized_body = Some(body);
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
