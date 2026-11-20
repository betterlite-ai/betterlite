use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::{Config, ProxyTrust};
use betterlite_core::cost::prepare::BillingTokenCounter as _;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::BundledCatalog;
use betterlite_models::decisions::Json;
use betterlite_tokenizer::count::runtime::runtime_counter;

use crate::spend::estimates::counter::TokenizerCounter;

const CONFIG: &str = "general_settings:\n  master_key: test-master\n";

fn counter() -> TokenizerCounter {
    let lookup: BTreeMap<String, String> = BTreeMap::new();
    let config: Config = ConfigLoader { lookup }.load(CONFIG).unwrap();
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    let client = Arc::new(ProxiedClient::outbound(&Arc::new(policy)).unwrap());
    let catalog = BundledCatalog::default().get();
    TokenizerCounter {
        counter: Arc::new(runtime_counter(&config, &client, &catalog).unwrap()),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn counts_text_with_the_bundled_vocabulary() {
    let counted = counter()
        .count_text("gemini-embedding-001", "hello world")
        .await
        .unwrap();
    assert_eq!(counted, 2);
}

#[tokio::test]
async fn an_empty_text_counts_no_tokens() {
    let counted = counter()
        .count_text("gemini-embedding-001", "")
        .await
        .unwrap();
    assert_eq!(counted, 0);
}

#[tokio::test]
async fn counts_a_message_with_more_than_one_token() {
    let messages: Vec<Json> =
        serde_json::from_str(r#"[{"role":"user","content":"hello world"}]"#).unwrap();
    let counted = counter()
        .count_messages("gemini-embedding-001", &messages)
        .await
        .unwrap();
    assert!(counted > 1);
}
