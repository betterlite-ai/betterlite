use betterlite_config::sections::mistral_resources::Settings;
use betterlite_openai::mistral_batches::records::MistralBatchJob;
use betterlite_openai::mistral_batches::transformation::{
    create_batch, retrieve_batch, to_batch_object,
};
use betterlite_openai::mistral_identity::auth_headers;
use betterlite_openai::openai_base_config::Headers;
use http::header::AUTHORIZATION;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::{BatchCreateCall, RetrieveCall};

pub(super) fn create(call: &BatchCreateCall, settings: &Settings) -> Value {
    let key = call.api_key.as_deref().map(SecretString::from);
    let Ok(headers) = auth_headers(&Headers::new(), key.as_ref(), settings) else {
        return json!({"ok": false, "status": null});
    };
    match create_batch(&call.data, &call.model, call.api_base.as_deref(), settings) {
        Ok(plan) => json!({
            "ok": true,
            "url": plan.url,
            "authorization": headers.get(AUTHORIZATION.as_str()),
            "body": plan.body,
        }),
        Err(error) => json!({"ok": false, "status": error.status_code()}),
    }
}

pub(super) fn job(job: &MistralBatchJob) -> Value {
    json!({"ok": true, "batch": to_batch_object(job)})
}

pub(super) fn retrieve(call: &RetrieveCall, settings: &Settings) -> Value {
    let key = call.api_key.as_deref().map(SecretString::from);
    match retrieve_batch(
        &call.batch_id,
        call.api_base.as_deref(),
        key.as_ref(),
        settings,
    ) {
        Ok(request) => json!({
            "ok": true,
            "method": request.method.as_str(),
            "url": request.url,
            "headers": request.headers,
        }),
        Err(error) => json!({"ok": false, "status": error.status_code()}),
    }
}
