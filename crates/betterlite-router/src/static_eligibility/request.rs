use betterlite_models::decisions::Json;

// What one request carries that a static filter reads; absent means the request did not send it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RequestFacts {
    pub messages: Option<Vec<Json>>,
    pub system: Option<Json>,
    pub tools: Vec<Json>,
    pub response_format: bool,
    pub target_order: Option<i64>,
    pub input_token_count: Option<usize>,
    pub compaction_pending: bool,
    pub drop_params: bool,
}
