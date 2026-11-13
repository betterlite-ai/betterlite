from collections.abc import Callable
from typing import TypeAlias, cast

import litellm
from litellm.proxy.policy_engine.condition_evaluator import ConditionEvaluator
from litellm.proxy.policy_engine.policy_matcher import PolicyMatcher
from litellm.proxy.policy_engine.policy_resolver import PolicyResolver
from litellm.types.proxy.policy_engine.pipeline_types import GuardrailPipeline
from litellm.types.proxy.policy_engine.policy_types import (
    Policy,
    PolicyAttachment,
    PolicyCondition,
    PolicyScope,
)
from litellm.types.proxy.policy_engine.resolver_types import PolicyMatchContext

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Handler: TypeAlias = Callable[[dict[str, Json]], Json]


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def string_list(value: Json, name: str) -> list[str]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return [string_value(item, name) for item in value]


def policy_condition(value: Json) -> PolicyCondition:
    return PolicyCondition(**object_value(value, "condition"))  # pyright: ignore[reportArgumentType]


def policy_context(value: Json) -> PolicyMatchContext:
    return PolicyMatchContext(**object_value(value, "context"))  # pyright: ignore[reportArgumentType]


def policy_scope(value: Json) -> PolicyScope:
    return PolicyScope(**object_value(value, "scope"))  # pyright: ignore[reportArgumentType]


def policy_attachment(value: Json) -> PolicyAttachment:
    return PolicyAttachment(**object_value(value, "attachment"))  # pyright: ignore[reportArgumentType]


def policy(name: str, value: Json) -> Policy:
    return Policy(**object_value(value, name))  # pyright: ignore[reportArgumentType]


def policy_map(value: Json) -> dict[str, Policy]:
    entries = object_value(value, "policies")
    return {name: policy(name, entry) for name, entry in entries.items()}


def optional_context(value: Json) -> PolicyMatchContext | None:
    if value is None:
        return None
    return policy_context(value)


def evaluate_condition(input_value: dict[str, Json]) -> Json:
    condition_value = input_value.get("condition")
    condition = None if condition_value is None else policy_condition(condition_value)
    context = policy_context(input_value.get("context"))
    matched = ConditionEvaluator.evaluate(condition=condition, context=context)
    return {"matches": matched}


def matches_pattern(input_value: dict[str, Json]) -> Json:
    patterns = string_list(input_value.get("patterns"), "patterns")
    value = input_value.get("value")
    if value is None:
        return {"matches": PolicyMatcher.matches_pattern(None, patterns)}
    return {"matches": PolicyMatcher.matches_pattern(string_value(value, "value"), patterns)}


def scope_matches(input_value: dict[str, Json]) -> Json:
    scope = policy_scope(input_value.get("scope"))
    context = policy_context(input_value.get("context"))
    return {"matches": PolicyMatcher.scope_matches(scope=scope, context=context)}


def inheritance_chain(input_value: dict[str, Json]) -> Json:
    chain = PolicyResolver.resolve_inheritance_chain(
        policy_name=string_value(input_value.get("policy_name"), "policy_name"),
        policies=policy_map(input_value.get("policies")),
    )
    return cast("Json", {"chain": chain})


def resolve_policy_guardrails(input_value: dict[str, Json]) -> Json:
    resolved = PolicyResolver.resolve_policy_guardrails(
        policy_name=string_value(input_value.get("policy_name"), "policy_name"),
        policies=policy_map(input_value.get("policies")),
        context=optional_context(input_value.get("context")),
    )
    return cast(
        "Json",
        {"guardrails": sorted(resolved.guardrails), "chain": resolved.inheritance_chain},
    )


def attachment_scope(input_value: dict[str, Json]) -> Json:
    scope = policy_attachment(input_value.get("attachment")).to_policy_scope()
    return cast(
        "Json",
        {
            "teams": scope.get_teams(),
            "keys": scope.get_keys(),
            "models": scope.get_models(),
            "tags": scope.get_tags(),
        },
    )


def validate_pipeline(input_value: dict[str, Json]) -> Json:
    pipeline = GuardrailPipeline(**object_value(input_value.get("pipeline"), "pipeline"))  # pyright: ignore[reportArgumentType]
    return {"valid": True, "steps": len(pipeline.steps)}


HANDLERS: dict[str, Handler] = {
    "evaluate_condition": evaluate_condition,
    "matches_pattern": matches_pattern,
    "scope_matches": scope_matches,
    "inheritance_chain": inheritance_chain,
    "resolve_policy_guardrails": resolve_policy_guardrails,
    "attachment_scope": attachment_scope,
    "validate_pipeline": validate_pipeline,
}


def run(input_value: dict[str, Json]) -> Json:
    function = string_value(input_value.get("function"), "function")
    handler = HANDLERS.get(function)
    if handler is None:
        raise ValueError(f"unknown {litellm.__name__} helper function {function}")
    return handler(input_value)
