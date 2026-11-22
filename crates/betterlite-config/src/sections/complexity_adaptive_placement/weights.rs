use serde::Deserialize;

use crate::loader::{ConfigError, EnvironmentLookup};
use crate::raw::RawNumber;
use crate::resolve::{parse_float, resolve_number};
use crate::sections::adaptive_classification::Weights;

#[derive(Debug, Default, Deserialize)]
pub(super) struct RawWeights {
    #[serde(default)]
    quality: Option<RawNumber>,
    #[serde(default)]
    cost: Option<RawNumber>,
}

impl RawWeights {
    pub(super) fn resolve<L>(self, prefix: &str, lookup: &L) -> Result<Weights, ConfigError>
    where
        L: EnvironmentLookup,
    {
        let quality = resolve_number(
            self.quality,
            &format!("{prefix}.quality"),
            lookup,
            parse_float,
        )?;
        let economy = resolve_number(self.cost, &format!("{prefix}.cost"), lookup, parse_float)?;
        Ok(Weights { quality, economy })
    }
}
