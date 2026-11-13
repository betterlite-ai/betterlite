use crate::fallback_categories::contracts::{
    ContentPolicyResponse, FallbackChain, OrdinaryFallbacks, RefusalRequest, RefusalResponse,
};

const CONTENT_FILTER: &str = "content_filter";
const ANTHROPIC_MESSAGES: &str = "anthropic_messages";

#[must_use]
pub fn has_default_fallbacks(model_groups: &[&str]) -> bool {
    for model_group in model_groups {
        if *model_group == "*" {
            return true;
        }
    }
    false
}

#[must_use]
pub fn has_content_policy_fallback(
    content_policy: &FallbackChain<'_>,
    router_fallbacks: &[&str],
) -> bool {
    match content_policy {
        FallbackChain::Absent => has_default_fallbacks(router_fallbacks),
        FallbackChain::Unresolved => false,
        FallbackChain::Resolved {
            has_unattempted, ..
        } => *has_unattempted,
    }
}

#[must_use]
pub fn refusal_fallback_available(request: &RefusalRequest<'_>) -> bool {
    if request.fallbacks_disabled {
        return false;
    }
    match &request.content_policy {
        FallbackChain::Absent => {}
        FallbackChain::Unresolved => return false,
        FallbackChain::Resolved {
            has_unattempted, ..
        } => return *has_unattempted,
    }
    if has_default_fallbacks(request.router_fallbacks) {
        return true;
    }
    match request.ordinary {
        OrdinaryFallbacks::Standard(chain) | OrdinaryFallbacks::NonStandard(chain) => match chain {
            FallbackChain::Resolved {
                has_unattempted, ..
            } => *has_unattempted,
            FallbackChain::Absent | FallbackChain::Unresolved => false,
        },
        OrdinaryFallbacks::Absent => false,
    }
}

#[must_use]
pub fn raises_content_policy_error(
    response: &ContentPolicyResponse<'_>,
    content_policy: &FallbackChain<'_>,
    router_fallbacks: &[&str],
) -> bool {
    if response.has_choices
        && response
            .first_finish_reason
            .is_none_or(|reason| reason != CONTENT_FILTER)
    {
        return false;
    }
    has_content_policy_fallback(content_policy, router_fallbacks)
}

#[must_use]
pub fn raises_anthropic_refusal(
    function_name: &str,
    response: RefusalResponse,
    request: &RefusalRequest<'_>,
) -> bool {
    if function_name != ANTHROPIC_MESSAGES {
        return false;
    }
    match response {
        RefusalResponse::RefusalWithStopDetails => refusal_fallback_available(request),
        RefusalResponse::NotRefusal | RefusalResponse::RefusalWithoutStopDetails => false,
    }
}
