use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;
use thiserror::Error;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::straiker::delivery::attempts::{PayloadError, RetryDelay, encode};
use crate::straiker::delivery::envelope::{self, DeliveryError, build, json_object};
use crate::straiker::delivery::failure_policy::record_verdict;
use crate::straiker::delivery::responses::{self, Verdict, VerdictAction};
use crate::straiker::delivery::settings::ApiVersion;
use crate::straiker::delivery::transport::WebhookPost;
use crate::straiker::delivery::v3_prepare::{PrepareError, prepare_v3};
use crate::straiker::delivery::{ApplyRequest, DEFAULT_BLOCK_MESSAGE, StraikerDelivery};
use crate::straiker_webhook::streamed::request_is_streamed;

// The body one webhook call sends, the headers it adds and the v3 replay scope it belongs to.
struct Prepared {
    body: String,
    extra: IndexMap<&'static str, String>,
    scope: String,
    prefixes: Vec<String>,
}

#[derive(Debug, Error)]
enum PlanError {
    #[error(transparent)]
    Delivery(#[from] DeliveryError),
    #[error(transparent)]
    Payload(#[from] PayloadError),
    #[error(transparent)]
    V3(#[from] PrepareError),
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
        let prepared = match self.prepare(request, &object) {
            Ok(prepared) => prepared,
            Err(error) => return self.fail(request, &error, false),
        };
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
        let reply = match self.post(&prepared.body, &prepared.extra).await {
            Ok(reply) => reply,
            Err(failure) => return self.fail(request, &failure.message, failure.is_unreachable),
        };
        let verdict = match responses::parse(self.settings.api_version, &reply.body) {
            Ok(verdict) => verdict,
            Err(failure) => return self.fail(request, &failure.message, false),
        };
        self.log_response(&reply);
        if self.settings.verbose {
            let action = verdict.action.wire_name();
            record_verdict(request.request_data, action, verdict.turn_id.as_deref());
        }
        if self.settings.api_version != ApiVersion::V1 {
            return self.enforce_v3(request, &prepared, &verdict);
        }
        warn_drift(&verdict);
        self.enforce_v1(request, &object, verdict)
    }

    fn prepare(&self, request: &ApplyRequest<'_>, object: &Object) -> Result<Prepared, PlanError> {
        let max = self.settings.max_payload_bytes;
        if self.settings.api_version == ApiVersion::V1 {
            let envelope = build(&self.settings, request, object)?;
            return Ok(Prepared {
                body: encode(&envelope, max)?,
                extra: IndexMap::new(),
                scope: String::new(),
                prefixes: Vec::new(),
            });
        }
        let prepared = prepare_v3(&self.settings, request, object)?;
        Ok(Prepared {
            body: encode(&prepared.payload, max)?,
            extra: prepared.extra,
            scope: prepared.scope,
            prefixes: prepared.prefixes,
        })
    }

    fn enforce_v1(
        &self,
        request: &ApplyRequest<'_>,
        object: &Object,
        verdict: Verdict,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        let message = verdict
            .blocked_reason
            .unwrap_or_else(|| DEFAULT_BLOCK_MESSAGE.to_owned());
        match verdict.action {
            VerdictAction::Blocked => Err(self.block(request, &message, true)),
            VerdictAction::Intervened => {
                let streamed = request.input_type == GuardrailInputType::Response
                    && request_is_streamed(object);
                let Some(texts) = verdict.texts.filter(|_| !streamed) else {
                    return Err(self.block(request, &message, true));
                };
                let mut inputs = request.inputs.clone();
                inputs.texts = Some(texts);
                Ok(inputs)
            }
            VerdictAction::None => Ok(request.inputs.clone()),
        }
    }

    fn enforce_v3(
        &self,
        request: &ApplyRequest<'_>,
        prepared: &Prepared,
        verdict: &Verdict,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        if verdict.action != VerdictAction::Blocked {
            return Ok(request.inputs.clone());
        }
        let message = verdict
            .blocked_reason
            .clone()
            .unwrap_or_else(|| DEFAULT_BLOCK_MESSAGE.to_owned());
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
}

// LiteLLM warns about a v1 answer whose schema_version is not the one this build sends; a v3 answer carries none.
fn warn_drift(verdict: &Verdict) {
    let Some(received) = verdict.schema_version.as_deref() else {
        return;
    };
    if received != envelope::SCHEMA_VERSION {
        tracing::warn!(
            expected = envelope::SCHEMA_VERSION,
            received,
            "straiker.schema_drift"
        );
    }
}
