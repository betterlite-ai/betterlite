use serde::{Deserialize, Serialize};

// The OpenAI SDK's two client classes; only construction differs for these endpoint calls
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    #[default]
    Sync,
    #[serde(rename = "async")]
    Asynchronous,
}

impl ClientKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Sync => "sync",
            Self::Asynchronous => "async",
        }
    }
}

pub struct SuppliedClient<'client, Client> {
    pub kind: ClientKind,
    pub transport: &'client Client,
}

#[cfg(test)]
mod tests;
