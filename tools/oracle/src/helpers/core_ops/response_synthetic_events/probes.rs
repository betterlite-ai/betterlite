use std::cell::Cell;
use std::collections::BTreeMap;
use std::str::FromStr;

use betterlite_core::api::response_synthetic_events::billing::UsageEstimate;
use betterlite_core::api::response_synthetic_events::stamping::{
    CostTarget, ResponsesCostCalculator,
};
use betterlite_openai::api::responses_objects::ResponseApiUsage;
use betterlite_oracle_helper_api::error::HelperError;
use rust_decimal::Decimal;

use super::support::{CostSpec, EstimateSpec, value_error};

pub(super) struct CostProbe {
    answer: Option<Decimal>,
    raises: bool,
    pub(super) calls: Cell<usize>,
}

impl CostProbe {
    pub(super) fn new(spec: Option<&CostSpec>) -> Result<Option<Self>, Box<HelperError>> {
        let Some(spec) = spec else {
            return Ok(None);
        };
        let (answer, raises) = match spec {
            CostSpec::Value { cost } => (
                Some(Decimal::from_str(cost).map_err(|error| value_error(error.to_string()))?),
                false,
            ),
            CostSpec::Null => (None, false),
            CostSpec::Raise => (None, true),
        };
        Ok(Some(Self {
            answer,
            raises,
            calls: Cell::new(0),
        }))
    }
}

impl ResponsesCostCalculator for CostProbe {
    type Error = std::io::Error;

    fn response_cost(&self, _target: &CostTarget<'_>) -> Result<Option<Decimal>, Self::Error> {
        self.calls.set(self.calls.get() + 1);
        if self.raises {
            return Err(std::io::Error::other("cost lookup failed"));
        }
        Ok(self.answer)
    }
}

pub(super) struct FixedEstimate {
    answer: Option<ResponseApiUsage>,
    pub(super) calls: Cell<usize>,
}

impl FixedEstimate {
    pub(super) fn new(spec: Option<&EstimateSpec>) -> Option<Self> {
        let answer = match spec? {
            EstimateSpec::Value {
                input_tokens,
                output_tokens,
            } => Some(ResponseApiUsage {
                input_tokens: *input_tokens,
                input_tokens_details: None,
                output_tokens: *output_tokens,
                output_tokens_details: None,
                total_tokens: input_tokens.saturating_add(*output_tokens),
                cost: None,
                extra: BTreeMap::new(),
            }),
            EstimateSpec::Empty => None,
        };
        Some(Self {
            answer,
            calls: Cell::new(0),
        })
    }
}

impl UsageEstimate for FixedEstimate {
    fn estimate(&self) -> Option<ResponseApiUsage> {
        self.calls.set(self.calls.get() + 1);
        self.answer.clone()
    }
}
