pub(crate) struct Op;

#[derive(serde::Deserialize)]
pub(crate) struct Input {
    function: String,
    body: Option<betterlite_anthropic::cache_plan::JsonObject>,
    caller_key_hash: Option<String>,
    deployment_id: Option<String>,
    provider_key: Option<String>,
    model: Option<String>,
    anthropic_version: Option<String>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let value = match input.function.as_str() {
            "cache_plan" => input
                .body
                .as_ref()
                .map(betterlite_anthropic::cache_plan::parse_cache_plan)
                .map_or_else(|| serde_json::json!({"kind": "unsupported", "reason": "unsupported_prompt_shape"}), plan),
            "prompt" => input
                .body
                .as_ref()
                .and_then(betterlite_anthropic::cache_plan::parse_prompt)
                .map_or_else(|| serde_json::Value::Null, |prefix| serde_json::json!({
                    "kind": "prompt",
                    "prefix_body": prefix.prefix_body,
                    "fingerprint": prefix.fingerprint,
                    "fingerprints": prefix.fingerprints,
                    "ttl_seconds": prefix.ttl_seconds,
                })),
            "cache_scope" => serde_json::json!({
                "kind": "scope",
                "fingerprint": betterlite_anthropic::cache_plan::cache_scope(
                    input.caller_key_hash.as_deref().unwrap_or_default(),
                    input.deployment_id.as_deref().unwrap_or_default(),
                    input.provider_key.as_deref().unwrap_or_default(),
                    input.model.as_deref().unwrap_or_default(),
                    input.anthropic_version.as_deref().unwrap_or("2023-06-01"),
                ),
            }),
            _ => return Self::not_built("anthropic_cache_plan"),
        };
        Ok(value)
    }
}

fn plan(
    result: Result<
        betterlite_anthropic::cache_plan::PromptCachePlan,
        betterlite_anthropic::cache_plan::UnsupportedCachePlan,
    >,
) -> serde_json::Value {
    match result {
        Ok(plan) => serde_json::json!({
            "kind": "plan",
            "full_body": plan.full_body,
            "breakpoints": plan.breakpoints.into_iter().map(|boundary| serde_json::json!({
                "fingerprint": boundary.fingerprint,
                "prefix_body": boundary.prefix_body,
                "ttl_seconds": boundary.ttl_seconds,
                "lookback_fingerprints": boundary.lookback_fingerprints,
                "content_fingerprint": boundary.content_fingerprint,
                "lookback_content_fingerprints": boundary.lookback_content_fingerprints,
            })).collect::<Vec<_>>(),
        }),
        Err(error) => serde_json::json!({
            "kind": "unsupported",
            "reason": reason(error.reason),
        }),
    }
}

const fn reason(reason: betterlite_anthropic::cache_plan::UnsupportedCacheReason) -> &'static str {
    match reason {
        betterlite_anthropic::cache_plan::UnsupportedCacheReason::UnsupportedPromptShape => {
            "unsupported_prompt_shape"
        }
        betterlite_anthropic::cache_plan::UnsupportedCacheReason::ConflictingCacheTtl => {
            "conflicting_cache_ttl"
        }
        betterlite_anthropic::cache_plan::UnsupportedCacheReason::TooManyCacheBreakpoints => {
            "too_many_cache_breakpoints"
        }
        betterlite_anthropic::cache_plan::UnsupportedCacheReason::InvalidCacheTtlOrder => {
            "invalid_cache_ttl_order"
        }
        betterlite_anthropic::cache_plan::UnsupportedCacheReason::UnsupportedThinkingCache => {
            "unsupported_thinking_cache_semantics"
        }
        betterlite_anthropic::cache_plan::UnsupportedCacheReason::TokenCountUnavailable => {
            "token_count_unavailable"
        }
        betterlite_anthropic::cache_plan::UnsupportedCacheReason::InconsistentPrefixTokenCount => {
            "inconsistent_prefix_token_count"
        }
    }
}
