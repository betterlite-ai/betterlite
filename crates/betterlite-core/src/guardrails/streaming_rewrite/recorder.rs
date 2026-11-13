use std::future::{Future, ready};
use std::marker::PhantomData;

use super::shapes::StreamInputs;
use super::traits::{GuardrailCall, NamedGuardrail, RewriteFailure, StreamGuardrail};

pub struct ScannedTextRecorder<Failure> {
    pub guardrail_name: String,
    pub inputs: Option<StreamInputs>,
    pub failure: PhantomData<fn() -> Failure>,
}

impl<Failure> ScannedTextRecorder<Failure> {
    #[must_use]
    pub const fn new(guardrail_name: String) -> Self {
        Self {
            guardrail_name,
            inputs: None,
            failure: PhantomData,
        }
    }
}

impl<Failure> NamedGuardrail for ScannedTextRecorder<Failure> {
    fn guardrail_name(&self) -> Option<&str> {
        Some(&self.guardrail_name)
    }
}

impl<Log: Send, Failure: RewriteFailure> StreamGuardrail<Log> for ScannedTextRecorder<Failure> {
    type Failure = Failure;

    fn covers_full_request(&self) -> bool {
        false
    }

    fn apply_guardrail(
        &mut self,
        inputs: StreamInputs,
        _call: GuardrailCall<'_, Log>,
    ) -> impl Future<Output = Result<StreamInputs, Self::Failure>> + Send {
        self.inputs = Some(inputs.clone());
        ready(Ok(inputs))
    }
}
