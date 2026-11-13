use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CountRow {
    #[serde(
        deserialize_with = "serde_with::As::<serde_with::PickFirst<(serde_with::Same, serde_with::DisplayFromStr)>>::deserialize"
    )]
    pub count: i64,
}

mod tests;
