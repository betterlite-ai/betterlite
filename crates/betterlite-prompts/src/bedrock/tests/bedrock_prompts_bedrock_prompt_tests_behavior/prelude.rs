use std::collections::BTreeMap;
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{Resolve, UrlValidator};

use crate::arguments::json::{Json, Object};
use crate::bedrock::assembly::{
    AssistantContinue, BedrockPrompt, PromptDeps, PromptSettings, bedrock_prompt,
};
use crate::bedrock::error::PromptError;
use crate::bedrock::flags::MessageCachePoints;
use crate::bedrock_images::remote::{RemoteImage, TRANSPORT_LIMITS};
use crate::bedrock_tool_results::invoke::CachePoints;
use crate::bedrock_tools::capabilities::{BedrockToolCapabilities, CachePointSupport};
use crate::message_content::Message;

pub(super) const GENERATED_ID: &str = "generated-id";

pub(super) fn json(raw: &str) -> Json {
    serde_json::from_str(raw).unwrap()
}

pub(super) fn list(raw: &str) -> Vec<Json> {
    serde_json::from_str(raw).unwrap()
}

pub(super) fn messages(raw: &str) -> Vec<Message> {
    serde_json::from_str(raw).unwrap()
}

pub(super) fn assistant_continue_with_call() -> AssistantContinue {
    let message = messages(
        r#"[{"role":"assistant","content":"x","tool_calls":[
            {"id":"c1","type":"function","function":{"name":"get weather","arguments":"{}"}}
        ]}]"#,
    )
    .into_iter()
    .next()
    .unwrap();
    AssistantContinue::Message(message)
}

pub(super) fn field<'value>(value: &'value Json, key: &str) -> Option<&'value Json> {
    let fields = value.as_object()?;
    fields.get(key)
}

pub(super) struct Settings<'call> {
    pub(super) model: Option<&'call str>,
    pub(super) capabilities: BedrockToolCapabilities,
    pub(super) cache_points: MessageCachePoints,
    pub(super) user_continue_message: Option<&'call Message>,
    pub(super) assistant_continue_message: Option<&'call AssistantContinue>,
    pub(super) modify_params: bool,
}

impl Default for Settings<'_> {
    fn default() -> Self {
        Self {
            model: None,
            capabilities: BedrockToolCapabilities::default(),
            cache_points: MessageCachePoints {
                support: CachePointSupport::Unsupported,
            },
            user_continue_message: None,
            assistant_continue_message: None,
            modify_params: false,
        }
    }
}

impl<'call> Settings<'call> {
    #[must_use]
    pub(super) fn with_model(mut self, model: &'call str) -> Self {
        self.model = Some(model);
        self
    }

    #[must_use]
    pub(super) fn with_modify_params(mut self) -> Self {
        self.modify_params = true;
        self
    }

    #[must_use]
    pub(super) fn with_user_continue(mut self, message: &'call Message) -> Self {
        self.user_continue_message = Some(message);
        self
    }

    #[must_use]
    pub(super) fn with_assistant_continue(mut self, message: &'call AssistantContinue) -> Self {
        self.assistant_continue_message = Some(message);
        self
    }

    #[must_use]
    pub(super) fn with_cache_points(mut self, support: CachePointSupport) -> Self {
        self.capabilities.cache_points = support;
        self.cache_points.support = support;
        self
    }

    #[must_use]
    pub(super) fn with_strict_tools(mut self) -> Self {
        self.capabilities.supports_strict_tools = true;
        self
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct RefusingResolver;

#[async_trait]
impl Resolve for RefusingResolver {
    async fn resolve(&self, host: &str, _port: u16) -> io::Result<Vec<SocketAddr>> {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("the test resolves no host, including {host}"),
        ))
    }
}

pub(super) struct Harness {
    remote: RemoteImage<RefusingResolver>,
}

impl Harness {
    pub(super) fn new() -> Self {
        let policy =
            Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
        let client = ProxiedClient::build(base_builder(), &policy, TRANSPORT_LIMITS).unwrap();
        let validator = UrlValidator::new(RefusingResolver, &[], &Verification::Enabled);
        Self {
            remote: RemoteImage {
                client,
                urls: UserUrlFetcher {
                    validator,
                    validation: true,
                },
                deadline: TRANSPORT_LIMITS.read_timeout,
                max_download_size_mb: 50.0,
            },
        }
    }

    pub(super) async fn convert(
        &self,
        messages: &[Message],
        tools: &[Json],
        settings: &Settings<'_>,
    ) -> Result<BedrockPrompt, PromptError> {
        self.convert_with(messages, tools, settings, &settings.cache_points)
            .await
    }

    pub(super) async fn convert_with<C>(
        &self,
        messages: &[Message],
        tools: &[Json],
        settings: &Settings<'_>,
        cache_points: &C,
    ) -> Result<BedrockPrompt, PromptError>
    where
        C: CachePoints + Sync,
    {
        let new_id = || GENERATED_ID.to_owned();
        let deps = PromptDeps {
            cache_points,
            remote: &self.remote,
            new_id: &new_id,
        };
        let prompt_settings = PromptSettings {
            model: settings.model,
            capabilities: settings.capabilities,
            user_continue_message: settings.user_continue_message,
            assistant_continue_message: settings.assistant_continue_message,
            modify_params: settings.modify_params,
        };
        bedrock_prompt(messages, tools, &prompt_settings, &deps).await
    }
}

pub(super) struct RecordingCachePoints {
    point: Option<Object>,
    seen: Mutex<Vec<(Json, Option<String>)>>,
}

impl RecordingCachePoints {
    pub(super) fn accepting() -> Self {
        Self {
            point: Some(Object::from_iter([(
                "type".to_owned(),
                Json::Text("default".to_owned()),
            )])),
            seen: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn seen(&self) -> Vec<(Json, Option<String>)> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CachePoints for RecordingCachePoints {
    fn cache_point_block(&self, cache_control: &Json, model: Option<&str>) -> Option<Object> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((cache_control.clone(), model.map(str::to_owned)));
        self.point.clone()
    }
}
