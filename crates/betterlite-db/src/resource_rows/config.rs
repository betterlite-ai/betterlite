use serde::{Deserialize, Serialize};

use crate::identity_rows::Payload;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigRow {
    pub param_name: String,
    pub param_value: Payload,
}
