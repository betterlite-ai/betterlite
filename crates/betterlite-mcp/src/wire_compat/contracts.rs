use std::future::Future;

use betterlite_prompts::arguments::json::Object;
use chrono::{DateTime, Utc};

use crate::server_record::MCPServer;

pub trait ProgressCallback {
    fn report(&self, progress: f64, total: Option<f64>) -> impl Future<Output = ()> + Send;
}

#[derive(Clone, Debug)]
pub struct AuthorizedToolCall<Callback: ProgressCallback> {
    pub name: String,
    pub arguments: Object,
    pub allowed_mcp_servers: Vec<MCPServer>,
    pub start_time: DateTime<Utc>,
    pub host_progress_callback: Option<Callback>,
    pub guardrail_context: Option<Object>,
    pub logging_data: Object,
}

mod tests;
