#![no_main]

use std::collections::BTreeMap;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{Resolve, UrlValidator};
use betterlite_prompts::bedrock::assembly::{PromptDeps, PromptSettings, bedrock_prompt};
use betterlite_prompts::bedrock::flags::MessageCachePoints;
use betterlite_prompts::bedrock::input::parse_prompt_bedrock_input;
use betterlite_prompts::bedrock_images::remote::{RemoteImage, TRANSPORT_LIMITS};
use betterlite_prompts::bedrock_tools::capabilities::{BedrockToolCapabilities, CachePointSupport};
use libfuzzer_sys::fuzz_target;
use tokio::runtime::Builder;

const GENERATED_ID: &str = "generated-id";

#[derive(Debug, Clone, Copy)]
struct RefusingResolver;

#[async_trait]
impl Resolve for RefusingResolver {
    async fn resolve(&self, host: &str, _port: u16) -> io::Result<Vec<SocketAddr>> {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("the fuzz input resolves no host, including {host}"),
        ))
    }
}

fn refusing_remote() -> Option<RemoteImage<RefusingResolver>> {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).ok()?);
    let client = ProxiedClient::build(base_builder(), &policy, TRANSPORT_LIMITS).ok()?;
    let validator = UrlValidator::new(RefusingResolver, &[], &Verification::Enabled);
    Some(RemoteImage {
        client,
        urls: UserUrlFetcher {
            validator,
            validation: true,
        },
        deadline: TRANSPORT_LIMITS.read_timeout,
        max_download_size_mb: 50.0,
    })
}

fuzz_target!(|bytes: &[u8]| {
    let Ok(input) = parse_prompt_bedrock_input(bytes) else {
        return;
    };
    let Some(remote) = refusing_remote() else {
        return;
    };
    let Ok(runtime) = Builder::new_current_thread().enable_all().build() else {
        return;
    };
    let cache_points = MessageCachePoints {
        support: CachePointSupport::from(input.capabilities.cache_points),
    };
    let settings = PromptSettings {
        model: input.model.as_deref(),
        capabilities: BedrockToolCapabilities::from(input.capabilities),
        user_continue_message: input.user_continue_message.as_ref(),
        assistant_continue_message: input.assistant_continue_message.as_ref(),
        modify_params: input.modify_params,
    };
    let new_id = || GENERATED_ID.to_owned();
    let deps = PromptDeps {
        cache_points: &cache_points,
        remote: &remote,
        new_id: &new_id,
    };
    drop(runtime.block_on(bedrock_prompt(
        &input.messages,
        &input.tools,
        &settings,
        &deps,
    )));
});
