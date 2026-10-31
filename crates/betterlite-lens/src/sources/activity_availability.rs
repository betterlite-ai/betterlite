use serde::{Deserialize, Serialize};

use crate::sources::row_decode::bool_from_bool_or_zero_one;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ActivityAvailability {
    #[serde(deserialize_with = "bool_from_bool_or_zero_one")]
    pub traces: bool,
    #[serde(deserialize_with = "bool_from_bool_or_zero_one")]
    pub requests: bool,
}

mod tests;
