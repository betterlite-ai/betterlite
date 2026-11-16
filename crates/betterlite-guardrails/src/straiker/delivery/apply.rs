use betterlite_prompts::arguments::json::Object;
use indexmap::IndexMap;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use crate::straiker::delivery::envelope::{self, build};
use crate::straiker::delivery::request_json::json_object;
use crate::straiker::delivery::responses::{self, Verdict, VerdictAction};
use crate::straiker::delivery::retry::RetryDelay;
use crate::straiker::delivery::settings::ApiVersion;
use crate::straiker::delivery::transport::WebhookPost;
use crate::straiker::delivery::v3_prepare::{V3Prepared, prepare_v3};
use crate::straiker::delivery::{ApplyRequest, DEFAULT_BLOCK_MESSAGE, StraikerDelivery};
use crate::straiker_webhook::streamed::request_is_streamed;

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
        prepared: &V3Prepared,
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
