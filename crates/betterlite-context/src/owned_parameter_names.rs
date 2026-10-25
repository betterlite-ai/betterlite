mod external_names;

use external_names::{CALLBACK, PRICING};

macro_rules! metadata {
    () => {
        None
    };
    ($name:literal) => {
        Some(WireMetadata { wire_name: $name })
    };
}
macro_rules! records {
    (roots { $($(#[$attribute:meta])* $root:ident { $($child:ident: $leaf:ident),* }),* }
     leaves { $($owner:ident { $($field:literal $(=> $wire:literal)?),* }),* }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum Owner { $($(#[$attribute])* $root,)* $($owner,)* }
        impl Owner {
            #[must_use]
            pub const fn record(self) -> Record<'static> {
                match self {
                    $(Self::$root => Record {
                        name: stringify!($root),
                        fields: &[$(Field { name: stringify!($child), metadata: None,
                            leaf: Some(Self::$leaf) }),*],
                    },)*
                    $(Self::$owner => Record {
                        name: stringify!($owner),
                        fields: &[$(Field { name: $field, metadata: metadata!($($wire)?),
                            leaf: None }),*],
                    },)*
                }
            }
        }
        const OWNED_LEAVES: &[Owner] = &[$($(Owner::$leaf,)*)*];
    };
}

records! {
    roots {
        ConnectionSettings { provider: ProviderConnection, bedrock_batch: BedrockBatchConnection },
        #[serde(rename = "litellm_options")]
        LiteLLMOptions { dispatch: DispatchOptions, routing: RoutingOptions, deployment: DeploymentOptions, specialized_routers: SpecializedRouterOptions, caching: CachingOptions, cost: CostOptions, observability: ObservabilityOptions, agentic_loop: AgenticLoopOptions, guardrails: GuardrailOptions, prompt: PromptOptions, response: ResponseOptions, control: ControlOptions, mock: MockOptions },
        InternalState { call: CallState, agentic_loop: AgenticLoopState, router: RouterState, proxy: ProxyRequestState, entrypoint: EntrypointState }
    }
    leaves {
        ProviderConnection { "api_key", "api_base", "api_version", "region_name", "headers", "provider_specific_header", "client", "shared_session", "ssl_verify", "request_timeout", "force_timeout", "stream_timeout", "max_retries", "tenant_id", "client_id", "client_secret", "azure_username", "azure_password", "azure_scope", "azure_ad_token_provider", "litellm_credential_name", "configurable_clientside_auth_params", "use_xai_oauth" },
        BedrockBatchConnection { "aws_batch_role_arn", "s3_bucket_name", "s3_region_name", "s3_endpoint_url", "s3_output_bucket_name", "s3_bucket_owner", "s3_access_key_id", "s3_secret_access_key", "s3_encryption_key_id", "bedrock_tags" },
        DispatchOptions { "custom_llm_provider", "azure", "use_litellm_proxy", "use_chat_completions_api", "use_in_pass_through", "allowed_openai_params" },
        RoutingOptions { "fallbacks", "context_window_fallback_dict", "num_retries", "retry_policy", "retry_strategy", "routing_strategy", "cooldown_time", "allowed_model_region", "enable_tag_filtering", "fastest_response", "provider_affinity_header", "search_tool_name", "model_list" },
        DeploymentOptions { "model_info", "rpm", "tpm", "itpm", "otpm", "default_api_key_rpm_limit", "default_api_key_tpm_limit", "max_parallel_requests", "weight", "order", "tag_regex", "max_file_size_mb", "silent_model" },
        SpecializedRouterOptions { "auto_router_config_path", "auto_router_config", "auto_router_default_model", "auto_router_embedding_model", "auto_router_max_input_chars", "auto_router_routing_compression", "auto_router_model_compression", "complexity_router_config", "complexity_router_default_model", "adaptive_router_config", "adaptive_router_default_model", "quality_router_config", "quality_router_default_model" },
        CachingOptions { "caching", "cache", "ttl", "enable_prompt_caching", "caching_groups" },
        CostOptions { "cost_per_query", "base_model", "max_budget", "budget_duration" },
        ObservabilityOptions { "id", "metadata", "litellm_metadata", "tags", "litellm_trace_id", "litellm_session_id", "litellm_request_debug", "logger_fn", "verbose", "no_log" => "no-log", "log_client_error_tracebacks" },
        AgenticLoopOptions { "max_agentic_loops" },
        GuardrailOptions { "guardrails" },
        PromptOptions { "prompt_id", "prompt_variables", "prompt_version", "prompt_environment", "prompt_label", "litellm_system_prompt", "custom_prompt_dict", "roles", "final_prompt_value", "bos_token", "eos_token", "hf_model_name", "supports_system_message", "ensure_alternating_roles", "user_continue_message", "assistant_continue_message", "disable_add_transform_inline_image_block" },
        ResponseOptions { "merge_reasoning_content_in_choices", "enable_json_schema_validation", "complete_response", "keepalive_seconds", "allow_client_keepalive_override" },
        ControlOptions { "stream_chunk_size" },
        MockOptions { "mock_response", "mock_timeout" },
        CallState { "litellm_call_id", "completion_call_id", "model_alias_map", "data_residency", "litellm_logging_obj", "preset_cache_key", "cache_key", "stream_response", "context_compaction_state" => "_context_compaction_state" },
        AgenticLoopState { "depth" => "_agentic_loop_depth", "fingerprints" => "_agentic_loop_fingerprints", "api_surface" => "_agentic_loop_api_surface", "code_interpreter_active" => "_code_interpreter_interception_active", "code_interpreter_sandbox_key" => "_code_interpreter_interception_sandbox_key", "code_interpreter_session_scoped" => "_code_interpreter_interception_session_scoped", "code_interpreter_converted_stream" => "_code_interpreter_interception_converted_stream", "websearch_emit_native_blocks" => "_websearch_interception_emit_native_blocks", "websearch_converted_stream" => "_websearch_interception_converted_stream", "headroom_converted_stream" => "_headroom_interception_converted_stream" },
        RouterState { "weights" => "_router_weights", "fallback_depth", "max_fallbacks", "attempted_targets" },
        ProxyRequestState { "proxy_server_request", "secret_fields", "trusted_callback_vars" => "litellm_trusted_callback_vars", "addressed_response_id" => "_litellm_addressed_response_id", "strip_stream_usage" => "_litellm_strip_stream_usage", "client_side_timeout", "model_file_id_mapping" },
        EntrypointState { "acompletion", "aembedding", "aimg_generation", "atext_completion", "text_completion", "allm_passthrough_route", "async_call" }
    }
}

pub const INTERNAL_KWARG_PREFIX: &str = "_litellm_";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct WireMetadata<'fields> {
    pub wire_name: &'fields str,
}

#[derive(Debug, Clone, Copy)]
pub struct Field<'fields> {
    pub name: &'static str,
    pub metadata: Option<WireMetadata<'fields>>,
    pub leaf: Option<Owner>,
}

#[derive(Debug, Clone, Copy)]
pub struct Record<'fields> {
    pub name: &'static str,
    pub fields: &'fields [Field<'fields>],
}

#[derive(Debug, thiserror::Error)]
pub enum OwnedParameterNamesError {
    #[error("{root}.{field} is not a dataclass leaf")]
    NotDataclassLeaf {
        root: &'static str,
        field: &'static str,
    },
}

#[must_use]
pub fn wire_names(owner: Owner) -> Vec<String> {
    let mut names = Vec::new();
    for field in owner.record().fields {
        names.push(
            field
                .metadata
                .map_or(field.name, |metadata| metadata.wire_name)
                .to_owned(),
        );
    }
    names
}

pub fn owned_wire_names(root: Record<'_>) -> Result<Vec<String>, OwnedParameterNamesError> {
    let mut names = Vec::new();
    for field in root.fields {
        let leaf = field
            .leaf
            .ok_or(OwnedParameterNamesError::NotDataclassLeaf {
                root: root.name,
                field: field.name,
            })?;
        names.extend(wire_names(leaf));
    }
    Ok(names)
}

#[derive(Debug, Clone, Copy)]
pub struct NameRegistry {
    owners: &'static [Owner],
}

impl NameRegistry {
    #[must_use]
    pub fn names(self) -> Vec<String> {
        let mut names = Vec::new();
        for owner in self.owners {
            names.extend(wire_names(*owner));
        }
        names
    }
}

pub const AGENTIC_LOOP_KWARG_NAMES: NameRegistry = NameRegistry {
    owners: &[Owner::AgenticLoopState, Owner::AgenticLoopOptions],
};
pub const BEDROCK_BATCH_KWARG_NAMES: NameRegistry = NameRegistry {
    owners: &[Owner::BedrockBatchConnection],
};

#[must_use]
pub fn owned_names() -> Vec<String> {
    let mut names = NameRegistry {
        owners: OWNED_LEAVES,
    }
    .names();
    names.extend(["self", "use_client", "model_config", "rust"].map(str::to_owned));
    names.extend(
        CALLBACK
            .iter()
            .chain(PRICING)
            .map(|name| (*name).to_owned()),
    );
    names
}

#[must_use]
pub fn is_litellm_owned_kwarg(name: &str) -> bool {
    if name.starts_with(INTERNAL_KWARG_PREFIX) {
        return true;
    }
    owned_names().iter().any(|owned| owned == name)
}

#[path = "owned_parameter_names/tests.rs"]
mod owned_parameter_names_cases;
