use std::fmt::Display;

use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;
use serde::Serialize;
use thiserror::Error;

use crate::generic_guardrail_contracts::{GuardrailInputType, UnreachableFallback};
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::guardrail_base::session::session_id_from_request;
use crate::straiker::delivery::block::{block_failure, block_message, record_verdict};
use crate::straiker::delivery::envelope::{self, build, response_shape};
use crate::straiker::delivery::headers::{self, WebhookHeaders};
use crate::straiker::delivery::payload;
use crate::straiker::delivery::request_json::{DeliveryError, json_object};
use crate::straiker::delivery::responses::{self, Verdict, VerdictAction};
use crate::straiker::delivery::retry::{RetryDelay, backoff_cap};
use crate::straiker::delivery::settings::{ApiVersion, FormatHint, StraikerSettings};
use crate::straiker::delivery::transport::{WebhookPost, WebhookReply, WebhookRequest};
use crate::straiker::delivery::{ApplyRequest, StraikerDelivery, phase_name};
use crate::straiker::v3::conversation_hash::conversation_prefixes;
use crate::straiker::v3::error::V3Error;
use crate::straiker::v3::header_composition::headers as v3_headers;
use crate::straiker::v3::payload_envelope::{PayloadInput, Phase, payload as detect_payload};
use crate::straiker::v3::request_body::provider_body;
use crate::straiker::v3::session_resolution::session_id;
use crate::straiker::v3::user_identity::user;
use crate::straiker_webhook::failure::{WebhookFailure, status_failure};
use crate::straiker_webhook::metadata::extract_identity;
use crate::straiker_webhook::streamed::request_is_streamed;
use crate::straiker_webhook::types::{StraikerWebhookContext, StraikerWebhookIdentity};

#[derive(Debug, Error)]
enum PrepareError {
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

struct V3Prepared {
    payload: Object,
    extra: IndexMap<&'static str, String>,
    scope: String,
    prefixes: Vec<String>,
}

impl<Post, Delay> StraikerDelivery<Post, Delay>
where
    Post: WebhookPost,
    Delay: RetryDelay,
{
    pub async fn apply(
        &self,
        request: &mut ApplyRequest<'_>,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        let object = match json_object(request.request_data) {
            Ok(object) => object,
            Err(error) => return self.fail(request, &error, false),
        };
        match self.settings.api_version {
            ApiVersion::V3 => self.apply_v3(request, &object).await,
            ApiVersion::V1 => self.apply_v1(request, &object).await,
        }
    }

    async fn apply_v1(
        &self,
        request: &mut ApplyRequest<'_>,
        object: &Object,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        let envelope = match build(&self.settings, request, object) {
            Ok(envelope) => envelope,
            Err(error) => return self.fail(request, &error, false),
        };
        let reply = match self.post(&envelope, &IndexMap::new()).await {
            Ok(reply) => reply,
            Err(failure) => return self.fail(request, &failure.message, failure.is_unreachable),
        };
        let verdict = match responses::parse(ApiVersion::V1, &reply.body) {
            Ok(verdict) => verdict,
            Err(failure) => return self.fail(request, &failure.message, false),
        };
        self.log_response(&reply);
        self.record(request, &verdict);
        warn_drift(&verdict);
        self.enforce_v1(request, object, verdict)
    }

    async fn apply_v3(
        &self,
        request: &mut ApplyRequest<'_>,
        object: &Object,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        let prepared = match prepare_v3(&self.settings, request, object) {
            Ok(prepared) => prepared,
            Err(error) => return self.fail(request, &error, false),
        };
        if request.input_type == GuardrailInputType::Request {
            let replayed = match self
                .blocked_turns
                .replayed(&prepared.scope, &prepared.prefixes)
            {
                Ok(replayed) => replayed,
                Err(error) => return self.fail(request, &error, false),
            };
            if let Some((prefix, message)) = replayed {
                self.log_replay(&prepared.scope, &prefix);
                return Err(self.block(request, &message, true));
            }
        }
        let reply = match self.post(&prepared.payload, &prepared.extra).await {
            Ok(reply) => reply,
            Err(failure) => return self.fail(request, &failure.message, failure.is_unreachable),
        };
        let verdict = match responses::parse(ApiVersion::V3, &reply.body) {
            Ok(verdict) => verdict,
            Err(failure) => return self.fail(request, &failure.message, false),
        };
        self.log_response(&reply);
        self.record(request, &verdict);
        self.enforce_v3(request, &prepared, &verdict)
    }

    fn enforce_v1(
        &self,
        request: &ApplyRequest<'_>,
        object: &Object,
        verdict: Verdict,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        match verdict.action {
            VerdictAction::Blocked => {
                let message = block_message(verdict.blocked_reason);
                Err(self.block(request, &message, true))
            }
            VerdictAction::Intervened => {
                let streamed = request.input_type == GuardrailInputType::Response
                    && request_is_streamed(object);
                if verdict.texts.is_none() || streamed {
                    let message = block_message(verdict.blocked_reason);
                    return Err(self.block(request, &message, true));
                }
                let mut inputs = request.inputs.clone();
                inputs.texts = verdict.texts;
                Ok(inputs)
            }
            VerdictAction::None => Ok(request.inputs.clone()),
        }
    }

    fn enforce_v3(
        &self,
        request: &ApplyRequest<'_>,
        prepared: &V3Prepared,
        verdict: &Verdict,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        if verdict.action != VerdictAction::Blocked {
            return Ok(request.inputs.clone());
        }
        let message = block_message(verdict.blocked_reason.clone());
        if request.input_type == GuardrailInputType::Request
            && let (Some(prefix), true) = (prepared.prefixes.last(), !verdict.blocked_by.is_empty())
            && let Err(error) = self
                .blocked_turns
                .remember(&prepared.scope, prefix, &message)
        {
            return self.fail(request, &error, false);
        }
        Err(self.block(request, &message, true))
    }

    async fn post<Value>(
        &self,
        payload: &Value,
        extra: &IndexMap<&'static str, String>,
    ) -> Result<WebhookReply, WebhookFailure>
    where
        Value: Serialize,
    {
        let body = payload::encode(payload, self.settings.max_payload_bytes).map_err(|error| {
            WebhookFailure {
                message: error.to_string(),
                is_unreachable: false,
                retryable: false,
            }
        })?;
        let url = self.settings.webhook_url();
        self.log_request(&url, &body);
        let merged = headers::compose(&self.settings, extra);
        let attempts = self.settings.max_retries.saturating_add(1);
        let mut attempt = 0_i64;
        loop {
            match self.deliver(&url, &body, &merged).await {
                Ok(reply) => return Ok(reply),
                Err(failure) => {
                    if !failure.retryable || attempt.saturating_add(1) >= attempts {
                        return Err(failure);
                    }
                    let cap = backoff_cap(
                        self.settings.initial_backoff,
                        self.settings.max_backoff,
                        attempt,
                    );
                    tokio::time::sleep(self.delay.delay(cap)).await;
                    attempt = attempt.saturating_add(1);
                }
            }
        }
    }

    async fn deliver(
        &self,
        url: &str,
        body: &str,
        headers: &WebhookHeaders,
    ) -> Result<WebhookReply, WebhookFailure> {
        let request = WebhookRequest {
            url,
            body,
            headers,
            timeout: self.settings.timeout,
        };
        let reply = self.transport.post(request).await?;
        if reply.status == 200 {
            return Ok(reply);
        }
        Err(status_failure(reply.status, &reply.body))
    }

    fn record(&self, request: &mut ApplyRequest<'_>, verdict: &Verdict) {
        if !self.settings.verbose {
            return;
        }
        record_verdict(
            request.request_data,
            verdict.action.wire_name(),
            verdict.turn_id.as_deref(),
        );
    }

    fn log_request(&self, url: &str, body: &str) {
        if !self.settings.verbose {
            return;
        }
        tracing::info!(
            event = "straiker.webhook_request",
            url = %url,
            bytes = body.len(),
            payload = %body,
            "straiker.webhook_request"
        );
    }

    fn log_response(&self, reply: &WebhookReply) {
        if !self.settings.verbose {
            return;
        }
        tracing::info!(
            event = "straiker.webhook_response",
            status_code = reply.status,
            body = %reply.body,
            "straiker.webhook_response"
        );
    }

    fn log_replay(&self, scope: &str, prefix: &str) {
        if !self.settings.verbose {
            return;
        }
        tracing::info!(
            event = "straiker.replay_blocked",
            scope = %scope,
            prefix = %prefix,
            "straiker.replay_blocked"
        );
    }

    fn block(
        &self,
        request: &ApplyRequest<'_>,
        message: &str,
        blocked_content: bool,
    ) -> GuardrailFailure {
        block_failure(
            &self.settings.guardrail_name,
            request.request_data,
            request.input_type,
            message,
            blocked_content,
        )
    }

    fn fail(
        &self,
        request: &ApplyRequest<'_>,
        error: &dyn Display,
        is_unreachable: bool,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        let fail_open = (is_unreachable
            && self.settings.unreachable_fallback == UnreachableFallback::FailOpen)
            || !self.settings.fail_on_error;
        tracing::error!(
            event = "straiker.error",
            input_type = phase_name(request.input_type),
            error = %error,
            fail_open,
            "straiker.error"
        );
        if fail_open {
            return Ok(request.inputs.clone());
        }
        let message = format!("Straiker detection unavailable: {error}");
        Err(self.block(request, &message, false))
    }
}

fn prepare_v3(
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

// LiteLLM warns about a v1 answer whose schema_version is not the one this build sends.
fn warn_drift(verdict: &Verdict) {
    let Some(received) = verdict
        .schema_version
        .as_deref()
        .filter(|version| *version != envelope::SCHEMA_VERSION)
    else {
        return;
    };
    tracing::warn!(
        expected = envelope::SCHEMA_VERSION,
        received,
        "straiker.schema_drift"
    );
}
