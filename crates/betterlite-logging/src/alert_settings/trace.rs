use std::time::Duration;

use betterlite_config::sections::langfuse_settings;
use tokio::time::sleep;

use crate::langfuse_settings::resolve_host;

const TRACE_ATTEMPTS: u8 = 3;
const TRACE_RETRY_WAIT: Duration = Duration::from_secs(3);

pub struct LangfuseTraceContext<'request> {
    pub langfuse_registered: bool,
    pub dynamic_host: Option<&'request str>,
    pub instance_host: Option<&'request str>,
}

pub trait LangfuseTraceSource: Send + Sync {
    fn trace_id(&self) -> Option<String>;
}

// LiteLLM's _add_langfuse_trace_id_to_alert: three lookups, three seconds apart, then no trace link
#[must_use]
pub async fn langfuse_trace_url(
    settings: &langfuse_settings::Settings,
    context: LangfuseTraceContext<'_>,
    source: Option<&dyn LangfuseTraceSource>,
) -> Option<String> {
    if !context.langfuse_registered {
        return None;
    }
    let trace_source = source?;
    let chosen = [context.dynamic_host, context.instance_host]
        .into_iter()
        .flatten()
        .find(|host| !host.is_empty());
    let host = resolve_host(settings, chosen);
    for _ in 0..TRACE_ATTEMPTS {
        if let Some(trace_id) = trace_source.trace_id() {
            return Some(format!("{host}/trace/{trace_id}"));
        }
        sleep(TRACE_RETRY_WAIT).await;
    }
    None
}
