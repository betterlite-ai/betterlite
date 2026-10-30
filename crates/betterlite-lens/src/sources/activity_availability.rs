use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ActivityAvailability {
    pub traces: bool,
    pub requests: bool,
}

mod tests;
