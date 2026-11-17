use std::str;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_prompts::arguments::json::{Json, Object};

use crate::agent_requests::endpoint::{AgentEndpoint, endpoint_url};
use crate::agent_requests::error::check_response;
use crate::agent_requests::prepare::PreparedAgentCall;
use crate::agent_results::messages::{MessagesData, extract_content};
use crate::agent_results::reply::{AgentReply, ReplyInput, TokenCounter, build_model_response};
use crate::agent_results::request::{INSTRUCTIONS_KEY, SYSTEM_ROLE, USER_ROLE};

use super::decode::{
    RUN_CANCELLED, RUN_COMPLETED, RUN_EXPIRED, RUN_FAILED, UNKNOWN_ERROR, decode_agent_messages,
    decode_agent_run_status, decode_created_id,
};
use super::error::AgentRunError;
use super::transport::{
    AgentRunClientFactory, AgentRunMethod, AgentRunMode, AgentRunRequest, AgentRunTransport,
    Sleeper,
};

pub const MAX_POLL_ATTEMPTS: usize = 60;
pub const POLL_INTERVAL: Duration = Duration::from_secs(1);
pub const RUN_AGENT_KEY: &str = "assistant_id";
pub const ROLE_KEY: &str = "role";
pub const CONTENT_KEY: &str = "content";
pub const NOT_TEXT_BODY: &str = "the response body is not UTF-8";
pub const CREATE_CODES: &[u16] = &[200, 201];
pub const READ_CODES: &[u16] = &[200];
pub const CREATE_THREAD_FAILED: &str = "Failed to create thread";
pub const ADD_MESSAGE_FAILED: &str = "Failed to add message";
pub const CREATE_RUN_FAILED: &str = "Failed to create run";
pub const RUN_STATUS_FAILED: &str = "Failed to get run status";
pub const FETCH_MESSAGES_FAILED: &str = "Failed to get messages";

#[derive(Debug, Clone, Copy)]
pub struct Expectation {
    pub codes: &'static [u16],
    pub prefix: &'static str,
}

pub const CREATE_THREAD: Expectation = Expectation {
    codes: CREATE_CODES,
    prefix: CREATE_THREAD_FAILED,
};
pub const ADD_MESSAGE: Expectation = Expectation {
    codes: CREATE_CODES,
    prefix: ADD_MESSAGE_FAILED,
};
pub const CREATE_RUN: Expectation = Expectation {
    codes: CREATE_CODES,
    prefix: CREATE_RUN_FAILED,
};
pub const RUN_STATUS: Expectation = Expectation {
    codes: READ_CODES,
    prefix: RUN_STATUS_FAILED,
};
pub const FETCH_MESSAGES: Expectation = Expectation {
    codes: READ_CODES,
    prefix: FETCH_MESSAGES_FAILED,
};

pub struct CompletionCall<'data> {
    pub model: &'data str,
    pub messages: &'data [Object],
    pub optional_params: &'data Object,
    pub prepared: PreparedAgentCall,
    pub ssl_verify: Json,
    pub reply: AgentReply,
    pub mode: AgentRunMode,
    pub supplied: Option<&'data dyn AgentRunTransport>,
}

struct Call<'data> {
    prepared: &'data PreparedAgentCall,
    client: &'data dyn AgentRunTransport,
    sleeper: &'data dyn Sleeper,
}

enum ClientRef<'data> {
    Supplied(&'data dyn AgentRunTransport),
    Created(Box<dyn AgentRunTransport>),
}

impl Call<'_> {
    async fn send(
        &self,
        method: AgentRunMethod,
        url: String,
        body: Option<&Object>,
        expectation: Expectation,
    ) -> Result<UpstreamResponse, AgentRunError> {
        let encoded = match body {
            Some(payload) => Some(serde_json::to_vec(payload).map_err(AgentRunError::Body)?),
            None => None,
        };
        let request = AgentRunRequest {
            method,
            url,
            headers: self.prepared.headers.clone(),
            body: encoded,
        };
        let response = self
            .client
            .send(request)
            .await
            .map_err(AgentRunError::Transport)?;
        if expectation.codes.contains(&response.status) {
            return Ok(response);
        }
        let text = str::from_utf8(&response.body).unwrap_or(NOT_TEXT_BODY);
        check_response(response.status, text, expectation.codes, expectation.prefix)?;
        Ok(response)
    }

    async fn create_thread(&self) -> Result<String, AgentRunError> {
        let url = endpoint_url(
            AgentEndpoint::Threads,
            &self.prepared.api_base,
            &self.prepared.api_version,
        )?;
        let response = self
            .send(AgentRunMethod::Post, url, None, CREATE_THREAD)
            .await?;
        Ok(decode_created_id(&response.body)?)
    }

    async fn add_messages(
        &self,
        thread_id: &str,
        messages: &[Object],
    ) -> Result<(), AgentRunError> {
        let url = endpoint_url(
            AgentEndpoint::Messages { thread_id },
            &self.prepared.api_base,
            &self.prepared.api_version,
        )?;
        for message in messages {
            let role = message.get(ROLE_KEY).and_then(Json::as_str);
            if role != Some(USER_ROLE) && role != Some(SYSTEM_ROLE) {
                continue;
            }
            let mut payload = Object::new();
            payload.insert(String::from(ROLE_KEY), Json::Text(String::from(USER_ROLE)));
            payload.insert(
                String::from(CONTENT_KEY),
                message
                    .get(CONTENT_KEY)
                    .cloned()
                    .unwrap_or_else(|| Json::Text(String::new())),
            );
            self.send(
                AgentRunMethod::Post,
                url.clone(),
                Some(&payload),
                ADD_MESSAGE,
            )
            .await?;
        }
        Ok(())
    }

    async fn create_run(
        &self,
        thread_id: &str,
        optional_params: &Object,
    ) -> Result<String, AgentRunError> {
        let url = endpoint_url(
            AgentEndpoint::Runs { thread_id },
            &self.prepared.api_base,
            &self.prepared.api_version,
        )?;
        let mut payload = Object::new();
        payload.insert(
            String::from(RUN_AGENT_KEY),
            Json::Text(self.prepared.agent_id.clone()),
        );
        if let Some(instructions) = optional_params.get(INSTRUCTIONS_KEY) {
            payload.insert(String::from(INSTRUCTIONS_KEY), instructions.clone());
        }
        let response = self
            .send(AgentRunMethod::Post, url, Some(&payload), CREATE_RUN)
            .await?;
        Ok(decode_created_id(&response.body)?)
    }

    async fn poll(&self, thread_id: &str, run_id: &str) -> Result<(), AgentRunError> {
        let url = endpoint_url(
            AgentEndpoint::RunStatus { thread_id, run_id },
            &self.prepared.api_base,
            &self.prepared.api_version,
        )?;
        for _ in 0..MAX_POLL_ATTEMPTS {
            let response = self
                .send(AgentRunMethod::Get, url.clone(), None, RUN_STATUS)
                .await?;
            let decoded = decode_agent_run_status(&response.body)?;
            let last_error = decoded.last_error;
            match decoded.status.as_deref() {
                Some(RUN_COMPLETED) => return Ok(()),
                Some(status @ (RUN_FAILED | RUN_CANCELLED | RUN_EXPIRED)) => {
                    let message = last_error
                        .and_then(|error| error.message)
                        .unwrap_or_else(|| String::from(UNKNOWN_ERROR));
                    return Err(AgentRunError::RunFailed {
                        status: String::from(status),
                        message,
                    });
                }
                Some(_) | None => {}
            }
            self.sleeper.sleep(POLL_INTERVAL).await;
        }
        Err(AgentRunError::TimedOut)
    }

    async fn fetch_messages(&self, thread_id: &str) -> Result<MessagesData, AgentRunError> {
        let url = endpoint_url(
            AgentEndpoint::Messages { thread_id },
            &self.prepared.api_base,
            &self.prepared.api_version,
        )?;
        let response = self
            .send(AgentRunMethod::Get, url, None, FETCH_MESSAGES)
            .await?;
        Ok(decode_agent_messages(&response.body)?)
    }
}

pub async fn completion(
    call: &CompletionCall<'_>,
    factory: &dyn AgentRunClientFactory,
    sleeper: &dyn Sleeper,
    counter: &dyn TokenCounter,
) -> Result<AgentReply, AgentRunError> {
    let held = call.supplied.map_or_else(
        || {
            ClientRef::Created(match call.mode {
                AgentRunMode::Sync => factory.sync_client(&call.ssl_verify),
                AgentRunMode::Async => factory.async_client(&call.ssl_verify),
            })
        },
        ClientRef::Supplied,
    );
    let client: &dyn AgentRunTransport = match &held {
        ClientRef::Supplied(supplied) => *supplied,
        ClientRef::Created(created) => created.as_ref(),
    };
    let flow = Call {
        prepared: &call.prepared,
        client,
        sleeper,
    };
    let thread_id = match call.prepared.thread_id.as_deref() {
        Some(thread_id) if !thread_id.is_empty() => String::from(thread_id),
        Some(_) | None => flow.create_thread().await?,
    };
    flow.add_messages(&thread_id, call.messages).await?;
    let run_id = flow.create_run(&thread_id, call.optional_params).await?;
    flow.poll(&thread_id, &run_id).await?;
    let messages = flow.fetch_messages(&thread_id).await?;
    let extracted = extract_content(&messages);
    let input = ReplyInput {
        model: call.model,
        content: &extracted.content,
        thread_id: &thread_id,
        messages: call.messages,
        annotations: extracted.annotations,
    };
    Ok(build_model_response(call.reply.clone(), &input, counter).await)
}
