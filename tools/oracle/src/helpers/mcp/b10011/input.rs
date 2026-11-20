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

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FailureMethod {
    Upsert,
    FindFirst,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Failure {
    pub(crate) method: FailureMethod,
    pub(crate) call: usize,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub(crate) enum Step {
    CacheSet {
        user_id: String,
        assertion: Box<AssertionInput>,
        seen_epoch: u64,
    },
    CacheGet {
        user_id: String,
    },
    Invalidate {
        user_id: String,
    },
    Persist {
        user_id: String,
        assertion: Box<AssertionInput>,
    },
    Retention,
    Retain {
        user_id: String,
        assertion: Option<Box<AssertionInput>>,
    },
}

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    pub(crate) database: bool,
    pub(crate) config_servers: Vec<Option<String>>,
    pub(crate) db_servers: Vec<Option<String>>,
    pub(crate) failure: Option<Failure>,
    pub(crate) steps: Vec<Step>,
}
