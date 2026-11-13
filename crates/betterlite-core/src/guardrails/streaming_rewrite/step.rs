use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::legacy::LegacyHookStreamAdapter;
use super::observer::StreamRewriteObserver;
use super::shapes::RequestData;
use super::traits::{
    GuardrailCallback, RewriteFailure as _, StreamCall, StreamGuardrail, StreamTranslation,
    UserApiKey,
};

pub struct StreamingStep<'step, Log> {
    pub guardrail: &'step str,
    pub streaming_chunks: &'step mut Vec<GuardrailValue>,
    pub hook_input: &'step mut RequestData,
    pub user_api_key_dict: &'step UserApiKey,
    pub logging_obj: Option<&'step mut Log>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepOutcome {
    Applied,
    Discarded { reason: String },
}

fn release_original_chunks(
    guardrail_name: &str,
    reason: &str,
    streaming_chunks: &mut Vec<GuardrailValue>,
    originals: Vec<GuardrailValue>,
) {
    *streaming_chunks = originals;
    tracing::warn!(
        guardrail = guardrail_name,
        reason,
        "guardrail rewrote the streamed response but the rewrite could not be written back to the stream; the whole rewrite was discarded and the original stream released"
    );
}

// record_applied is add_guardrail_to_applied_guardrails_header, which the callback utilities own
pub async fn run_streaming_step<Log, Callback, Translation>(
    callback: &mut Callback,
    endpoint_translation: &Translation,
    mut step: StreamingStep<'_, Log>,
    record_applied: fn(&mut RequestData, &str),
) -> Result<StepOutcome, Callback::Failure>
where
    Log: Send,
    Callback: GuardrailCallback<Log>,
    Translation: StreamTranslation<Log, Failure = Callback::Failure>,
{
    let records_own = callback.records_own_information();
    let outcome = if callback.supports_unified_execution() {
        observe_step(callback, endpoint_translation, &mut step).await?
    } else {
        let mut adapter = LegacyHookStreamAdapter {
            inner: callback,
            endpoint_translation,
            user_api_key_dict: step.user_api_key_dict,
        };
        observe_step(&mut adapter, endpoint_translation, &mut step).await?
    };
    if outcome == StepOutcome::Applied && !records_own {
        record_applied(step.hook_input, step.guardrail);
    }
    Ok(outcome)
}

async fn observe_step<Log, Scanner, Translation>(
    scanner: &mut Scanner,
    endpoint_translation: &Translation,
    step: &mut StreamingStep<'_, Log>,
) -> Result<StepOutcome, Scanner::Failure>
where
    Log: Send,
    Scanner: StreamGuardrail<Log>,
    Translation: StreamTranslation<Log, Failure = Scanner::Failure>,
{
    let mut observer = StreamRewriteObserver::new(scanner);
    let originals = step.streaming_chunks.clone();
    step.hook_input.remove("response");
    let call = StreamCall {
        request_data: Some(&mut *step.hook_input),
        user_api_key_dict: step.user_api_key_dict,
        logging_obj: step.logging_obj.as_deref_mut(),
        deliver_ended_stream_rewrites: Translation::DELIVERS_ENDED_STREAM_REWRITES,
    };
    let result = endpoint_translation
        .process_streaming_response(&mut *step.streaming_chunks, &mut observer, call)
        .await;
    if let Err(error) = result {
        let Some(undeliverable) = error.undeliverable() else {
            return Err(error);
        };
        let reason = undeliverable.reason.clone();
        release_original_chunks(
            step.guardrail,
            &reason,
            &mut *step.streaming_chunks,
            originals,
        );
        return Ok(StepOutcome::Discarded { reason });
    }
    let Some(reason) = observer.discard_reason(Translation::DELIVERS_ENDED_STREAM_REWRITES) else {
        return Ok(StepOutcome::Applied);
    };
    release_original_chunks(
        step.guardrail,
        &reason,
        &mut *step.streaming_chunks,
        originals,
    );
    Ok(StepOutcome::Discarded { reason })
}
