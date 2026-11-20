use betterlite_models::decisions::Json;

use crate::shadow_requests::Object;

use super::ChatRequestFuture;

const MESSAGES: &str = "messages";

#[must_use]
pub fn chat_request<'request>(
    kwargs: &'request Object,
    model_parameters: &'request Object,
) -> ChatRequestFuture<'request> {
    Box::pin(async move {
        let mut request = model_parameters.clone();
        request.insert(MESSAGES.to_owned(), Json::List(messages(kwargs)));
        Ok(request)
    })
}

fn messages(kwargs: &Object) -> Vec<Json> {
    match kwargs.get(MESSAGES) {
        Some(Json::List(items)) => items
            .iter()
            .filter(|item| matches!(item, Json::Object(_)))
            .cloned()
            .collect(),
        None
        | Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            Vec::new()
        }
    }
}
