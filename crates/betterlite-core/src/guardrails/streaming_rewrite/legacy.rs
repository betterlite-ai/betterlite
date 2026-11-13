use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::failure::UndeliverableStreamRewrite;
use super::recorder::ScannedTextRecorder;
use super::shapes::{RequestData, StreamInputs, tool_call_mismatch_reason};
use super::traits::{
    GuardrailCall, GuardrailCallback, NamedGuardrail, PostCallHook, StreamCall, StreamGuardrail,
    StreamTranslation, UserApiKey,
};

pub struct LegacyHookStreamAdapter<'adapter, Callback, Translation> {
    pub inner: &'adapter mut Callback,
    pub endpoint_translation: &'adapter Translation,
    pub user_api_key_dict: &'adapter UserApiKey,
}

impl<Callback: NamedGuardrail, Translation> NamedGuardrail
    for LegacyHookStreamAdapter<'_, Callback, Translation>
{
    fn guardrail_name(&self) -> Option<&str> {
        self.inner.guardrail_name()
    }
}

impl<Log, Callback, Translation> StreamGuardrail<Log>
    for LegacyHookStreamAdapter<'_, Callback, Translation>
where
    Log: Send,
    Callback: GuardrailCallback<Log>,
    Translation: StreamTranslation<Log, Failure = Callback::Failure>,
{
    type Failure = Callback::Failure;

    fn covers_full_request(&self) -> bool {
        self.inner.covers_full_request()
    }

    async fn apply_guardrail(
        &mut self,
        inputs: StreamInputs,
        call: GuardrailCall<'_, Log>,
    ) -> Result<StreamInputs, Self::Failure> {
        let GuardrailCall {
            request_data,
            logging_obj,
            ..
        } = call;
        let replacement = self.run_post_call_hook(request_data).await?;
        let Some(rewrite) = replacement else {
            return Ok(inputs);
        };
        let guardrail_name = self.guardrail_name().unwrap_or("unknown").to_owned();
        let Some(rescanned) = rescan(
            self.endpoint_translation,
            self.user_api_key_dict,
            rewrite,
            logging_obj,
            &guardrail_name,
        )
        .await?
        else {
            return Err(<Self::Failure>::from(UndeliverableStreamRewrite {
                guardrail_name,
                reason: "the legacy hook's response could not be rescanned by this endpoint's translation"
                    .to_owned(),
            }));
        };
        if let Some(reason) = undeliverable_reason(&inputs, &rescanned) {
            return Err(<Self::Failure>::from(UndeliverableStreamRewrite {
                guardrail_name,
                reason,
            }));
        }
        match rescanned.texts {
            Some(texts) if !texts.is_empty() => Ok(StreamInputs {
                texts: Some(texts),
                ..inputs
            }),
            _ => Ok(inputs),
        }
    }
}

impl<Callback, Translation> LegacyHookStreamAdapter<'_, Callback, Translation> {
    async fn run_post_call_hook<Log>(
        &mut self,
        request_data: &mut RequestData,
    ) -> Result<Option<GuardrailValue>, Callback::Failure>
    where
        Log: Send,
        Callback: GuardrailCallback<Log>,
        Translation: StreamTranslation<Log, Failure = Callback::Failure>,
    {
        let stored_response = request_data
            .get("response")
            .filter(|value| !matches!(value, GuardrailValue::Null))
            .cloned();
        let mut hooked = self
            .endpoint_translation
            .post_call_hook_response(stored_response);
        let replacement = self
            .inner
            .async_post_call_success_hook(PostCallHook {
                data: request_data,
                user_api_key_dict: self.user_api_key_dict,
                response: &mut hooked,
            })
            .await?;
        Ok(replacement.or(hooked))
    }
}

fn undeliverable_reason(sent: &StreamInputs, rescanned: &StreamInputs) -> Option<String> {
    let returned_text_count = rescanned.texts.as_ref().map_or(0, Vec::len);
    let sent_text_count = sent.texts.as_ref().map_or(0, Vec::len);
    if returned_text_count != sent_text_count {
        return Some(format!(
            "the legacy hook returned {returned_text_count} texts for a stream that carried {sent_text_count}"
        ));
    }
    tool_call_mismatch_reason(
        sent.tool_shapes().as_deref(),
        rescanned.tool_shapes().as_deref(),
    )
}

async fn rescan<Log, Translation>(
    endpoint_translation: &Translation,
    user_api_key_dict: &UserApiKey,
    response: GuardrailValue,
    logging_obj: Option<&mut Log>,
    guardrail_name: &str,
) -> Result<Option<StreamInputs>, Translation::Failure>
where
    Log: Send,
    Translation: StreamTranslation<Log>,
{
    let mut recorder = ScannedTextRecorder::<Translation::Failure>::new(guardrail_name.to_owned());
    let call = StreamCall {
        request_data: None,
        user_api_key_dict,
        logging_obj,
        deliver_ended_stream_rewrites: false,
    };
    endpoint_translation
        .process_output_response(response, &mut recorder, call)
        .await?;
    Ok(recorder.inputs)
}
