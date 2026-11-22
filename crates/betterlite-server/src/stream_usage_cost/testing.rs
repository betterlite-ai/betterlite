use betterlite_core::cost::recording::RecordInput;
use betterlite_logging::fields::usage::Usage;
use betterlite_logging::logging_state::LoggingState;
use rust_decimal::Decimal;

use super::{StreamCostCalculator, StreamCostRecorder};

pub struct FixedRecorder(pub Option<Decimal>);

pub struct FixedCalculator(pub Option<Decimal>);

impl StreamCostRecorder for FixedRecorder {
    fn response_cost(&self, _log: &mut LoggingState, _input: RecordInput<'_>) -> Option<Decimal> {
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
