import asyncio

from litellm.proxy.agent_endpoints import agent_registry

from .limiter import (
    AGENT,
    Operations,
    TagLimits,
    make_auth,
    make_limiter,
    tag_limits_of,
)
from .types import (
    AssembleCall,
    CreateCall,
    CreatePolicy,
    DescriptorAnswer,
    DynamicAnswer,
    DynamicFlagCall,
    EnforceAnswer,
    EnforcedLimitCall,
    LimitAnswer,
    ShouldEnforceCall,
)


def dynamic_flag_answer(call: DynamicFlagCall) -> DynamicAnswer:
    limiter = make_limiter({}, TagLimits({}))
    dynamic = Operations.dynamic_flag(
        limiter, call["rpm_limit_type"], call["tpm_limit_type"]
    )
    return DynamicAnswer(dynamic=dynamic)


def enforce_answer(call: ShouldEnforceCall) -> EnforceAnswer:
    limiter = make_limiter({}, TagLimits({}))
    enforce = Operations.enforce(
        limiter, call["limit_type"], model_has_failures=call["model_has_failures"]
    )
    return EnforceAnswer(enforce=enforce)


def limit_answer(call: EnforcedLimitCall) -> LimitAnswer:
    limiter = make_limiter({}, TagLimits({}))
    limit = Operations.enforced_limit(
        limiter,
        call["limit_value"],
        call["limit_type"],
        model_has_failures=call["model_has_failures"],
    )
    return LimitAnswer(limit=limit)


def descriptor_answer(call: CreateCall | AssembleCall) -> DescriptorAnswer:
    agent_registry.global_agent_registry.agent_list = [
        AGENT.validate_python(entry) for entry in call["registry"]
    ]
    limiter = make_limiter(call["groups"], TagLimits(tag_limits_of(call["tag_limits"])))
    limiter.window_size = call["window_size"]
    auth = make_auth(call["auth"], call["server"])
    if call["function"] == "create":
        policy = CreatePolicy(
            rpm_limit_type=call["rpm_limit_type"],
            tpm_limit_type=call["tpm_limit_type"],
            model_has_failures=call["model_has_failures"],
        )
        return Operations.create(limiter, auth, call["data"], policy, call["call_type"])
    return asyncio.run(
        Operations.assembled(limiter, auth, call["data"], call["call_type"])
    )
