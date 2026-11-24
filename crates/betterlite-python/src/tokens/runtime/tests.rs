#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_tokenizer::engine::selection::TokenizerType;
use pyo3_async_runtimes::tokio::get_runtime;

use super::TokensRuntime;

fn runtime_for(source: &str) -> TokensRuntime {
    let environment = BTreeMap::new();
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &environment).unwrap();
    let client = Arc::new(ProxiedClient::outbound(&Arc::new(policy)).unwrap());
    let config = ConfigLoader {
        lookup: environment,
    }
    .load(source)
    .unwrap();
    TokensRuntime::compose(&config, &client).unwrap()
}

fn selected(runtime: &TokensRuntime) -> TokenizerType {
    let (_, kind) = get_runtime()
        .block_on(runtime.selector.select("claude-sonnet-4-5", None))
        .unwrap();
    kind
}

// the bundled catalog lists claude-sonnet-4-5 as anthropic without claude-3 in its name, so only the download flag decides this branch
#[test]
fn tokens_runtime_takes_the_download_flag_from_the_engines_section() {
    assert_eq!(selected(&runtime_for("{}")), TokenizerType::HuggingFace);
    let disabled = runtime_for("litellm_settings: {disable_hf_tokenizer_download: true}");
    assert_eq!(selected(&disabled), TokenizerType::OpenAi);
}
