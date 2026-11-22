use betterlite_server::shadow_result_aggregation::aggregation::pct_of;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use crate::router_probes::ProbeOutput;
use crate::router_probes::error::OperationError;
use crate::router_probes::random::InjectedRandom;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub pairs: Vec<Pair>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Pair {
    pub numerator: i64,
    pub denominator: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub rates: Vec<Rate>,
}

// a rate as the case spells it, compared bit for bit, and read through Number because the tagged reply buffers an arbitrary_precision float as its private number map
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Rate(pub f64);

// ast-grep-ignore: no-custom-serde
impl<'de> Deserialize<'de> for Rate {
    // ast-grep-ignore: no-custom-serde
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let number = serde_json::Number::deserialize(deserializer)?;
        number
            .as_f64()
            .map(Self)
            .ok_or_else(|| D::Error::custom("a rate must be a finite JSON number"))
    }
}

impl PartialEq for Rate {
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for Rate {}

pub(crate) fn run(
    input: &Input,
    _random: &mut InjectedRandom<'_>,
) -> Result<ProbeOutput, OperationError> {
    let mut rates = Vec::with_capacity(input.pairs.len());
    for pair in &input.pairs {
        let rate = pct_of(pair.numerator, pair.denominator).map_err(|error| {
            OperationError::InvalidRequest {
                message: error.to_string(),
            }
        })?;
        rates.push(Rate(rate));
    }
    Ok(ProbeOutput::ShadowResultAggregation(Output { rates }))
}
