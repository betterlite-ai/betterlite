use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;
use thiserror::Error;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::session::session_id_from_request;
use crate::straiker::delivery::ApplyRequest;
use crate::straiker::delivery::envelope::{self, response_shape};
use crate::straiker::delivery::request_json::DeliveryError;
use crate::straiker::delivery::settings::{FormatHint, StraikerSettings};
use crate::straiker::v3::conversation_hash::conversation_prefixes;
use crate::straiker::v3::error::V3Error;
use crate::straiker::v3::header_composition::headers as v3_headers;
use crate::straiker::v3::payload_envelope::{PayloadInput, Phase, payload as detect_payload};
use crate::straiker::v3::request_body::provider_body;
use crate::straiker::v3::session_resolution::session_id;
use crate::straiker::v3::user_identity::user;
use crate::straiker_webhook::metadata::extract_identity;
use crate::straiker_webhook::types::{StraikerWebhookContext, StraikerWebhookIdentity};

#[derive(Debug, Error)]
pub enum PrepareError {
    #[error(transparent)]
    Delivery(#[from] DeliveryError),
    #[error(transparent)]
    V3(#[from] V3Error),
}

struct V3Parts {
    context: StraikerWebhookContext,
    identity: StraikerWebhookIdentity,
    texts: Vec<String>,
    request_body: Object,
}

// The detect payload, its extra headers and the replay scope of one v3 call.
pub struct V3Prepared {
    pub payload: Object,
    pub extra: IndexMap<&'static str, String>,
    pub scope: String,
    pub prefixes: Vec<String>,
}

pub fn prepare_v3(
    settings: &StraikerSettings,
    request: &ApplyRequest<'_>,
    object: &Object,
) -> Result<V3Prepared, PrepareError> {
    let parts = v3_parts(settings, request, object);
    let payload = detect_body(request, &parts, object)?;
    let extra = v3_headers(
        object,
        settings.agent_ref.as_deref(),
        settings.client.as_deref(),
        settings.format_hint.map(FormatHint::as_str),
    );
    let (scope, prefixes) = conversation_scope(&parts, object)?;
    Ok(V3Prepared {
        payload,
        extra,
        scope,
        prefixes,
    })
}

fn v3_parts(settings: &StraikerSettings, request: &ApplyRequest<'_>, object: &Object) -> V3Parts {
    let model = envelope::model_of(&request.inputs, object);
    let context = envelope::context(
        settings,
        request.call,
        object,
        model.as_deref(),
        session_id_from_request(request.request_data),
    );
    let texts = request.inputs.texts.clone().unwrap_or_default();
    let request_texts: &[String] = match request.input_type {
        GuardrailInputType::Request => &texts,
        GuardrailInputType::Response => &[],
    };
    let request_body = provider_body(
        object,
        request_texts,
        request.call.routes,
        request.call.decoder,
    );
    V3Parts {
        context,
        identity: extract_identity(object),
        texts,
        request_body,
    }
}

fn detect_body(
    request: &ApplyRequest<'_>,
    parts: &V3Parts,
    object: &Object,
) -> Result<Object, PrepareError> {
    let response = response_shape(object)?;
    let phase = match request.input_type {
        GuardrailInputType::Request => Phase::Request,
        GuardrailInputType::Response => Phase::Response,
    };
    let input = PayloadInput {
        context: &parts.context,
        identity: &parts.identity,
        texts: &parts.texts,
        response: &response,
        request_data: object,
        routes: request.call.routes,
        translator: request.call.translator,
    };
    Ok(detect_payload(&input, phase, &parts.request_body)?)
}

fn conversation_scope(
    parts: &V3Parts,
    object: &Object,
) -> Result<(String, Vec<String>), PrepareError> {
    let session = session_id(&parts.context, &parts.identity, object, &parts.request_body)?;
    let principal = user(&parts.identity).or_else(|| parts.identity.litellm_key.clone());
    let scope = match (&principal, &session) {
        (None, None) => String::new(),
        _ => {
            let principal_text = principal.as_deref().unwrap_or_default();
            let session_text = session.as_deref().unwrap_or_default();
            format!("{principal_text}\0{session_text}")
        }
    };
    if scope.is_empty() {
        return Ok((scope, Vec::new()));
    }
    let prefixes = conversation_prefixes(&parts.request_body)?;
    Ok((scope, prefixes))
}
