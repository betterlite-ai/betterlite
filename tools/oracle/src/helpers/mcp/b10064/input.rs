use serde::{Deserialize, Serialize};

// The fields a case declares for an assertion; the assertion itself comes back through decode_payload.
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct AssertionInput {
    pub(crate) id_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) issuer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) expires_at: Option<String>,
}

// The ciphertext is stored as given, so a case can seed the exact row LiteLLM wrote.
#[derive(Debug, Deserialize)]
pub(crate) struct SeedRow {
    pub(crate) user_id: String,
    pub(crate) assertion_b64: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FailureMethod {
    Find,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Failure {
    pub(crate) method: FailureMethod,
    pub(crate) call: usize,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Step {
    Fetch {
        user_id: String,
        #[serde(default)]
        invalidate_during_read: bool,
    },
    FetchUncached {
        user_id: String,
    },
    Persist {
        user_id: String,
        assertion: Box<AssertionInput>,
    },
}

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    pub(crate) database: bool,
    pub(crate) rows: Vec<SeedRow>,
    pub(crate) failure: Option<Failure>,
    pub(crate) steps: Vec<Step>,
}
