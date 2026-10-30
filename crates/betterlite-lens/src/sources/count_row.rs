use serde::{Deserialize, Serialize};

use crate::sources::row_decode::i64_from_int_or_numeric_string;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CountRow {
    #[serde(deserialize_with = "i64_from_int_or_numeric_string")]
    pub count: i64,
}

mod tests;
