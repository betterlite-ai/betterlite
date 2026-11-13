from typing import Literal, TypeAlias, TypedDict

from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    RateLimitedModel,
    fail_closed_rate_limit_enforcement_enabled,
)
from pydantic import JsonValue


class FlagConfig(TypedDict):
    general_settings: dict[str, JsonValue]


class CounterKeysInput(TypedDict):
    function: Literal["counter_keys"]
    key: str
    value: str
    rate_limit_type: Literal["requests", "tokens", "max_parallel_requests"]


class SiblingCounterKeysInput(TypedDict):
    function: Literal["sibling_counter_keys"]
    window_key: str


class ModelLimitInput(TypedDict):
    function: Literal["model_limit"]
    requested: str
    group: str
    limits: dict[str, int] | None


class FailClosedFlagInput(TypedDict):
    function: Literal["fail_closed_flag"]
    config: FlagConfig


Input: TypeAlias = (
    CounterKeysInput | SiblingCounterKeysInput | ModelLimitInput | FailClosedFlagInput
)


def run(input_value: Input) -> dict[str, JsonValue]:
    if input_value["function"] == "model_limit":
        model = RateLimitedModel(
            requested=input_value["requested"], group=input_value["group"]
        )
        return {"limit": model.limit_in(input_value["limits"])}
    if input_value["function"] == "fail_closed_flag":
        return {
            "enabled": fail_closed_rate_limit_enforcement_enabled(
                input_value["config"]["general_settings"]
            )
        }
    raise ValueError(
        "the key helpers have no public LiteLLM entry, so a case cannot compare them"
    )
