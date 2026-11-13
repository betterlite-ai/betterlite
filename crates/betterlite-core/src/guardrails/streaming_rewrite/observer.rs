use super::shapes::{StreamInputs, count_change, rewrote};
use super::traits::{GuardrailCall, NamedGuardrail, StreamGuardrail};

pub struct StreamRewriteObserver<'inner, Inner> {
    pub inner: &'inner mut Inner,
    pub rewrote_texts: bool,
    pub rewrote_tool_calls: bool,
    pub tool_call_count_change: Option<(usize, usize)>,
}

impl<'inner, Inner> StreamRewriteObserver<'inner, Inner> {
    pub const fn new(inner: &'inner mut Inner) -> Self {
        Self {
            inner,
            rewrote_texts: false,
            rewrote_tool_calls: false,
            tool_call_count_change: None,
        }
    }

    #[must_use]
    pub fn discard_reason(&self, deliver_rewrites: bool) -> Option<String> {
        if let Some((sent, returned)) = self.tool_call_count_change {
            return Some(format!(
                "the guardrail returned {returned} tool calls for a stream that carried {sent}, and a rewrite that drops or adds a tool call cannot be written back"
            ));
        }
        if !deliver_rewrites && (self.rewrote_texts || self.rewrote_tool_calls) {
            return Some(
                "this endpoint's streaming pipeline does not write ended-stream rewrites back yet"
                    .to_owned(),
            );
        }
        None
    }
}

impl<Inner: NamedGuardrail> NamedGuardrail for StreamRewriteObserver<'_, Inner> {
    fn guardrail_name(&self) -> Option<&str> {
        self.inner.guardrail_name()
    }
}

impl<Log: Send, Inner: StreamGuardrail<Log>> StreamGuardrail<Log>
    for StreamRewriteObserver<'_, Inner>
{
    type Failure = Inner::Failure;

    fn covers_full_request(&self) -> bool {
        self.inner.covers_full_request()
    }

    async fn apply_guardrail(
        &mut self,
        inputs: StreamInputs,
        call: GuardrailCall<'_, Log>,
    ) -> Result<StreamInputs, Self::Failure> {
        let sent_texts = inputs.texts.clone();
        let sent_shapes = inputs.tool_shapes();
        let outputs = self.inner.apply_guardrail(inputs, call).await?;
        let returned_shapes = outputs.tool_shapes();
        self.rewrote_texts =
            self.rewrote_texts || rewrote(sent_texts.as_deref(), outputs.texts.as_deref());
        self.rewrote_tool_calls =
            self.rewrote_tool_calls || rewrote(sent_shapes.as_deref(), returned_shapes.as_deref());
        self.tool_call_count_change = self
            .tool_call_count_change
            .or_else(|| count_change(sent_shapes.as_deref(), returned_shapes.as_deref()));
        Ok(outputs)
    }
}
