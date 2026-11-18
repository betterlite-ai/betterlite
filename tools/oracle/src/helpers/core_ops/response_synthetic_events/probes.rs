use std::cell::Cell;
use std::str::FromStr;

use betterlite_core::api::response_synthetic_events::billing::UsageEstimate;
use betterlite_core::api::response_synthetic_events::stamping::{
    CostTarget, ResponsesCostCalculator,
};
use betterlite_openai::api::responses_objects::ResponseApiUsage;
use betterlite_oracle_helper_api::error::HelperError;
use rust_decimal::Decimal;

use super::support::{CostSpec, value_error};

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
        self.calls.set(self.calls.get().saturating_add(1));
        if self.raises {
            return Err(std::io::Error::other("cost lookup failed"));
        }
        Ok(self.answer)
    }
}

pub(super) struct FixedEstimate {
    pub(super) answer: Option<ResponseApiUsage>,
    pub(super) calls: Cell<usize>,
}

impl UsageEstimate for FixedEstimate {
    fn estimate(&self) -> Option<ResponseApiUsage> {
        self.calls.set(self.calls.get().saturating_add(1));
        self.answer.clone()
    }
}
