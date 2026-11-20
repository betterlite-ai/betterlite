use serde::Deserialize;

// A plain row is encrypted under the salt key before it is stored; a raw row is stored as given.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum RowSeed {
    Plain { user_id: String, payload: String },
    Raw { user_id: String, text: String },
}

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    pub(crate) rows: Vec<RowSeed>,
}
