use betterlite_oracle_helper_api::group::Group;

// every group crate is an optional dependency and the feature of the same name, so a check builds only the groups its cases call
pub const GROUPS: &[Group] = &[
    #[cfg(feature = "betterlite-oracle-a2a")]
    betterlite_oracle_a2a::GROUP,
    #[cfg(feature = "betterlite-oracle-anthropic")]
    betterlite_oracle_anthropic::GROUP,
    #[cfg(feature = "betterlite-oracle-auth")]
    betterlite_oracle_auth::GROUP,
    #[cfg(feature = "betterlite-oracle-azure")]
    betterlite_oracle_azure::GROUP,
    #[cfg(feature = "betterlite-oracle-bedrock")]
    betterlite_oracle_bedrock::GROUP,
    #[cfg(feature = "betterlite-oracle-cache")]
    betterlite_oracle_cache::GROUP,
    #[cfg(feature = "betterlite-oracle-cli")]
    betterlite_oracle_cli::GROUP,
    #[cfg(feature = "betterlite-oracle-config")]
    betterlite_oracle_config::GROUP,
    #[cfg(feature = "betterlite-oracle-context")]
    betterlite_oracle_context::GROUP,
    #[cfg(feature = "betterlite-oracle-core-ops")]
    betterlite_oracle_core_ops::GROUP,
    #[cfg(feature = "betterlite-oracle-cost")]
    betterlite_oracle_cost::GROUP,
    #[cfg(feature = "betterlite-oracle-db")]
    betterlite_oracle_db::GROUP,
    #[cfg(feature = "betterlite-oracle-error")]
    betterlite_oracle_error::GROUP,
    #[cfg(feature = "betterlite-oracle-foundry")]
    betterlite_oracle_foundry::GROUP,
    #[cfg(feature = "betterlite-oracle-guardrails")]
    betterlite_oracle_guardrails::GROUP,
    #[cfg(feature = "betterlite-oracle-harness")]
    betterlite_oracle_harness::GROUP,
    #[cfg(feature = "betterlite-oracle-http")]
    betterlite_oracle_http::GROUP,
    #[cfg(feature = "betterlite-oracle-lens")]
    betterlite_oracle_lens::GROUP,
    #[cfg(feature = "betterlite-oracle-logging")]
    betterlite_oracle_logging::GROUP,
    #[cfg(feature = "betterlite-oracle-mcp")]
    betterlite_oracle_mcp::GROUP,
    #[cfg(feature = "betterlite-oracle-models")]
    betterlite_oracle_models::GROUP,
    #[cfg(feature = "betterlite-oracle-ocr")]
    betterlite_oracle_ocr::GROUP,
    #[cfg(feature = "betterlite-oracle-openai")]
    betterlite_oracle_openai::GROUP,
    #[cfg(feature = "betterlite-oracle-openapi")]
    betterlite_oracle_openapi::GROUP,
    #[cfg(feature = "betterlite-oracle-paths")]
    betterlite_oracle_paths::GROUP,
    #[cfg(feature = "betterlite-oracle-policy")]
    betterlite_oracle_policy::GROUP,
    #[cfg(feature = "betterlite-oracle-prometheus")]
    betterlite_oracle_prometheus::GROUP,
    #[cfg(feature = "betterlite-oracle-prompts")]
    betterlite_oracle_prompts::GROUP,
    #[cfg(feature = "betterlite-oracle-provider")]
    betterlite_oracle_provider::GROUP,
    #[cfg(feature = "betterlite-oracle-proxy")]
    betterlite_oracle_proxy::GROUP,
    #[cfg(feature = "betterlite-oracle-pyhost")]
    betterlite_oracle_pyhost::GROUP,
    #[cfg(feature = "betterlite-oracle-rate-limit")]
    betterlite_oracle_rate_limit::GROUP,
    #[cfg(feature = "betterlite-oracle-redact")]
    betterlite_oracle_redact::GROUP,
    #[cfg(feature = "betterlite-oracle-redis")]
    betterlite_oracle_redis::GROUP,
    #[cfg(feature = "betterlite-oracle-reporting")]
    betterlite_oracle_reporting::GROUP,
    #[cfg(feature = "betterlite-oracle-roi")]
    betterlite_oracle_roi::GROUP,
    #[cfg(feature = "betterlite-oracle-sdk")]
    betterlite_oracle_sdk::GROUP,
    #[cfg(feature = "betterlite-oracle-secret-store")]
    betterlite_oracle_secret_store::GROUP,
    #[cfg(feature = "betterlite-oracle-secrets")]
    betterlite_oracle_secrets::GROUP,
    #[cfg(feature = "betterlite-oracle-server")]
    betterlite_oracle_server::GROUP,
    #[cfg(feature = "betterlite-oracle-streaming")]
    betterlite_oracle_streaming::GROUP,
    #[cfg(feature = "betterlite-oracle-tokenizer")]
    betterlite_oracle_tokenizer::GROUP,
    #[cfg(feature = "betterlite-oracle-vertex")]
    betterlite_oracle_vertex::GROUP,
    #[cfg(feature = "betterlite-oracle-websearch")]
    betterlite_oracle_websearch::GROUP,
];

mod tests;
