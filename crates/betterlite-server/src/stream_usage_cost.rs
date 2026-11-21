mod error;
mod frame;
mod pricing;
mod projection;
mod tests;

use std::borrow::Cow;
use std::str::from_utf8;

use betterlite_core::cost::recording::RecordInput;
use betterlite_logging::fields::usage::Usage;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use rust_decimal::Decimal;

// Optional inspection must not parse a buffer of any size, so one call reads at most this much.
pub const MAX_INSPECTED_BYTES: usize = 1_048_576;

// B1717's _response_cost_calculator, bound by the caller that owns the catalog and calculator.
pub trait StreamCostRecorder: Send + Sync {
    fn response_cost(&self, log: &mut LoggingState, input: RecordInput<'_>) -> Option<Decimal>;
}

// B068's completion_cost, the model-name fallback when the call's own recorder yields nothing.
pub trait StreamCostCalculator: Send + Sync {
    fn completion_cost(
        &self,
        usage: &Usage,
        model_name: &str,
        service_tier: Option<&str>,
    ) -> Option<Decimal>;
}

#[derive(Clone, Copy)]
pub struct StreamCostInjector<'services> {
    pub enabled: bool,
    pub recorder: &'services dyn StreamCostRecorder,
    pub calculator: &'services dyn StreamCostCalculator,
}

impl StreamCostInjector<'_> {
    #[must_use]
    pub fn process_object<'object>(
        &self,
        object: &'object Json,
        model_name: &str,
        log: Option<&mut LoggingState>,
    ) -> Cow<'object, Json> {
        if !self.enabled {
            return Cow::Borrowed(object);
        }
        match projection::inject_into_object(object, model_name, *self, log) {
            Ok(Some(modified)) => Cow::Owned(modified),
            Ok(None) | Err(_) => Cow::Borrowed(object),
        }
    }

    #[must_use]
    pub fn process_text<'text>(
        &self,
        text: &'text str,
        model_name: &str,
        log: Option<&mut LoggingState>,
    ) -> Cow<'text, str> {
        if !self.enabled || text.len() > MAX_INSPECTED_BYTES {
            return Cow::Borrowed(text);
        }
        match frame::inject_into_frame(text, model_name, *self, log) {
            Some(rewritten) if rewritten.ends_with("\n\n") => Cow::Owned(rewritten),
            Some(rewritten) => Cow::Owned(format!("{rewritten}\n\n")),
            None => Cow::Borrowed(text),
        }
    }

    #[must_use]
    pub fn process_bytes<'bytes>(
        &self,
        bytes: &'bytes [u8],
        model_name: &str,
        log: Option<&mut LoggingState>,
    ) -> Cow<'bytes, [u8]> {
        if !self.enabled || bytes.len() > MAX_INSPECTED_BYTES {
            return Cow::Borrowed(bytes);
        }
        decode_cost_frame(*self, bytes, model_name, log)
    }
}

// The external byte decoder: only a complete UTF-8 frame is decoded and inspected.
#[must_use]
pub fn decode_cost_frame<'bytes>(
    injector: StreamCostInjector<'_>,
    frame: &'bytes [u8],
    model_name: &str,
    log: Option<&mut LoggingState>,
) -> Cow<'bytes, [u8]> {
    if !injector.enabled || frame.len() > MAX_INSPECTED_BYTES {
        return Cow::Borrowed(frame);
    }
    let Ok(text) = from_utf8(frame) else {
        return Cow::Borrowed(frame);
    };
    if !(text.ends_with("\n\n") || text.ends_with("\r\n\r\n")) {
        return Cow::Borrowed(frame);
    }
    match frame::inject_into_frame(text, model_name, injector, log) {
        Some(rewritten) => Cow::Owned(rewritten.into_bytes()),
        None => Cow::Borrowed(frame),
    }
}

#[cfg(any(test, feature = "testing"))]
pub mod testing {
    use betterlite_core::cost::recording::RecordInput;
    use betterlite_logging::fields::usage::Usage;
    use betterlite_logging::logging_state::LoggingState;
    use rust_decimal::Decimal;

    use super::{StreamCostCalculator, StreamCostRecorder};

    pub struct FixedRecorder(pub Option<Decimal>);

    pub struct FixedCalculator(pub Option<Decimal>);

    impl StreamCostRecorder for FixedRecorder {
        fn response_cost(
            &self,
            _log: &mut LoggingState,
            _input: RecordInput<'_>,
        ) -> Option<Decimal> {
            self.0
        }
    }

    impl StreamCostCalculator for FixedCalculator {
        fn completion_cost(
            &self,
            _usage: &Usage,
            _model_name: &str,
            _service_tier: Option<&str>,
        ) -> Option<Decimal> {
            self.0
        }
    }
}
