from typing import Literal, Protocol, TypeAlias, TypedDict, runtime_checkable

from litellm.proxy.hooks import PROXY_HOOKS
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    RateLimitedModel,
    fail_closed_rate_limit_enforcement_enabled,
)
from pydantic import JsonValue, TypeAdapter

LimitType: TypeAlias = Literal["requests", "tokens", "max_parallel_requests"]


class FlagConfig(TypedDict):
    general_settings: dict[str, JsonValue]


class CounterKeysInput(TypedDict):
    function: Literal["counter_keys"]
    key: str
    value: str
    rate_limit_type: LimitType


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


@runtime_checkable
class KeyBuilder(Protocol):
    def create_rate_limit_keys(
        self, key: str, value: str, rate_limit_type: LimitType
    ) -> str: ...


def run(input_value: Input) -> dict[str, JsonValue]:
    if input_value["function"] == "counter_keys":
        hook_class = TypeAdapter(type[KeyBuilder]).validate_python(
            PROXY_HOOKS["parallel_request_limiter"]
        )
        limiter = object.__new__(hook_class)
        return {
            "counter_key": limiter.create_rate_limit_keys(
                input_value["key"], input_value["value"], input_value["rate_limit_type"]
            )
        }
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
        "no public LiteLLM entry reaches the sibling counter key helper"
    )
