use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ActivityAvailability {
    #[serde(
        deserialize_with = "serde_with::As::<serde_with::PickFirst<(serde_with::Same, serde_with::BoolFromInt)>>::deserialize"
    )]
    pub traces: bool,
    #[serde(
        deserialize_with = "serde_with::As::<serde_with::PickFirst<(serde_with::Same, serde_with::BoolFromInt)>>::deserialize"
    )]
    pub requests: bool,
}

mod tests;
