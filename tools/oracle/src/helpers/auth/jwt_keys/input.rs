use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub(super) scenarios: Vec<Scenario>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Scenario {
    pub(super) settings: Settings,
    pub(super) steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Settings {
    pub(super) public_key_url: Option<String>,
    pub(super) public_key_ttl: f64,
    pub(super) public_key_stale_ttl: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Step {
    pub(super) now: f64,
    pub(super) kid: Option<String>,
    #[serde(default)]
    pub(super) http: Vec<Reply>,
    // two lookups enter the refresh together, so the lock and its recheck are what the case observes
    #[serde(default)]
    pub(super) concurrent: bool,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reply {
    pub(super) kind: ReplyKind,
    #[serde(default)]
    pub(super) status: u16,
    #[serde(default)]
    pub(super) body: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ReplyKind {
    Answer,
    Transport,
}
