use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;
use thiserror::Error;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::straiker::delivery::ApplyRequest;
use crate::straiker::delivery::envelope::{self, DeliveryError, response_shape};
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
    let payload = detect_payload(&input, phase, &parts.request_body)?;
    let extra = v3_headers(
        object,
        settings.agent_ref.as_deref(),
        settings.client.as_deref(),
        settings.format_hint.map(FormatHint::as_str),
    );
    let (scope, mut prefixes) = conversation_scope(&parts, object)?;
    if request.input_type != GuardrailInputType::Request {
        prefixes.clear();
    }
    Ok(V3Prepared {
        payload,
        extra,
        scope,
        prefixes,
    })
}

fn v3_parts(settings: &StraikerSettings, request: &ApplyRequest<'_>, object: &Object) -> V3Parts {
    let context = envelope::context(settings, request, object);
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

fn conversation_scope(parts: &V3Parts, object: &Object) -> Result<(String, Vec<String>), V3Error> {
    let session = session_id(&parts.context, &parts.identity, object, &parts.request_body)?;
    let principal = user(&parts.identity).or_else(|| parts.identity.litellm_key.clone());
    if principal.is_none() && session.is_none() {
        return Ok((String::new(), Vec::new()));
    }
    let scope = format!(
        "{}\0{}",
        principal.as_deref().unwrap_or_default(),
        session.as_deref().unwrap_or_default()
    );
    Ok((scope, conversation_prefixes(&parts.request_body)?))
}
