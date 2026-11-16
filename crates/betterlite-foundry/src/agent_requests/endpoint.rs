use betterlite_http::url_path::encode_segment;

use super::error::AgentRequestError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentEndpoint<'data> {
    Threads,
    Messages {
        thread_id: &'data str,
    },
    Runs {
        thread_id: &'data str,
    },
    RunStatus {
        thread_id: &'data str,
        run_id: &'data str,
    },
    ThreadAndRun,
}

// The builders append their path to the resolved base with no /openai prefix, and listing messages reuses Messages
pub fn endpoint_url(
    endpoint: AgentEndpoint<'_>,
    api_base: &str,
    api_version: &str,
) -> Result<String, AgentRequestError> {
    let path = match endpoint {
        AgentEndpoint::Threads => String::from("/threads"),
        AgentEndpoint::Messages { thread_id } => {
            format!(
                "/threads/{}/messages",
                encode_segment(thread_id, "thread_id")?
            )
        }
        AgentEndpoint::Runs { thread_id } => {
            format!("/threads/{}/runs", encode_segment(thread_id, "thread_id")?)
        }
        AgentEndpoint::RunStatus { thread_id, run_id } => format!(
            "/threads/{}/runs/{}",
            encode_segment(thread_id, "thread_id")?,
            encode_segment(run_id, "run_id")?
        ),
        AgentEndpoint::ThreadAndRun => String::from("/threads/runs"),
    };
    Ok(format!("{api_base}{path}?api-version={api_version}"))
}
