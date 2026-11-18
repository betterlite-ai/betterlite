use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    FromDb,
    EndUserFromDb,
    IncrementInMemory,
    Coalesced,
    WindowFromTable,
    WindowFromDb,
    WindowFromSpendLogs,
}

// a scripted answer is a number, "miss" or "fail": serde reads a unit variant as a bare string
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Word {
    Miss,
    Fail,
}

#[derive(Deserialize, Clone)]
#[serde(untagged)]
pub(crate) enum Answer {
    Amount(f64),
    Word(Word),
}

#[derive(Deserialize, Default, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SetNx {
    #[default]
    Stored,
    Taken,
    Fail,
}

// A bool on the wire; the newtype keeps struct_excessive_bools off the case input.
#[derive(Deserialize, Default, Clone, Copy)]
#[serde(transparent)]
pub(crate) struct Flag(pub(crate) bool);

#[derive(Deserialize, Default)]
pub(crate) struct CacheInput {
    #[serde(default)]
    pub(crate) redis: bool,
    #[serde(default)]
    pub(crate) answers: Vec<Answer>,
    #[serde(default)]
    pub(crate) set_nx: SetNx,
    #[serde(default)]
    pub(crate) memory: Option<f64>,
    #[serde(default)]
    pub(crate) memory_set_fails: bool,
}

#[derive(Deserialize)]
pub(crate) struct Input {
    pub(crate) op: Operation,
    #[serde(default)]
    pub(crate) counter_key: String,
    #[serde(default)]
    pub(crate) prisma: Flag,
    #[serde(default)]
    pub(crate) row_spend: Option<f64>,
    #[serde(default)]
    pub(crate) entity: String,
    #[serde(default)]
    pub(crate) entity_id: String,
    #[serde(default)]
    pub(crate) duration: Option<String>,
    #[serde(default)]
    pub(crate) start: String,
    #[serde(default)]
    pub(crate) expected_start: String,
    #[serde(default)]
    pub(crate) window: Option<(String, f64)>,
    #[serde(default)]
    pub(crate) logs: Vec<(String, f64)>,
    #[serde(default)]
    pub(crate) fails: Flag,
    #[serde(default)]
    pub(crate) cache: CacheInput,
    #[serde(default)]
    pub(crate) batch: Option<Answer>,
    #[serde(default)]
    pub(crate) increment: f64,
    #[serde(default)]
    pub(crate) require_cache_warm: Flag,
}
