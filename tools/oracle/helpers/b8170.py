import os
from datetime import datetime
from decimal import Decimal
from typing import Literal, NotRequired, TypedDict

import litellm
from litellm.litellm_core_utils.get_llm_provider_logic import get_llm_provider
from litellm.llms.dashscope.chat.transformation import DashScopeChatConfig
from litellm.llms.dashscope.cost_calculator import cost_per_token
from litellm.types.utils import Usage
from pydantic import JsonValue

CHAT_MODEL = "dashscope/probe"
COST_MODEL_PREFIX = "b8170-probe"


class Policy(TypedDict):
    default_base: str
    env_base: NotRequired[str | None]
    env_in_complete_url: bool


class SupportedParamsStep(TypedDict):
    kind: Literal["supported_params"]
    model: str
    chat_models: NotRequired[list[str]]
    text_models: NotRequired[list[str]]


class ProviderInfoStep(TypedDict):
    kind: Literal["provider_info"]
    policy: Policy
    api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]
    env_api_key: NotRequired[str | None]


class UsageStep(TypedDict):
    prompt_tokens: int
    completion_tokens: NotRequired[int]
    cached_tokens: NotRequired[int]
    cache_creation_tokens: NotRequired[int]
    reasoning_tokens: NotRequired[int]


class CostStep(TypedDict):
    kind: Literal["cost"]
    model_info: dict[str, JsonValue]
    usage: UsageStep
    current_time: str


Step = SupportedParamsStep | ProviderInfoStep | CostStep


class Input(TypedDict):
    steps: list[Step]


def set_env(name: str, value: str | None) -> None:
    if value is None:
        if name in os.environ:
            del os.environ[name]
    else:
        os.environ[name] = value


def build_usage(step: UsageStep) -> Usage:
    details: dict[str, int] = {}
    cached = step.get("cached_tokens")
    if cached is not None:
        details["cached_tokens"] = cached
    creation = step.get("cache_creation_tokens")
    if creation is not None:
        details["cache_creation_tokens"] = creation
    reasoning_details: dict[str, int] = {}
    reasoning = step.get("reasoning_tokens")
    if reasoning is not None:
        reasoning_details["reasoning_tokens"] = reasoning
    prompt_details: dict[str, int] | None = None
    if len(details) > 0:
        prompt_details = details
    completion_details: dict[str, int] | None = None
    if len(reasoning_details) > 0:
        completion_details = reasoning_details
    completion = step.get("completion_tokens")
    return Usage(
        prompt_tokens=step["prompt_tokens"],
        completion_tokens=0 if completion is None else completion,
        prompt_tokens_details=prompt_details,
        completion_tokens_details=completion_details,
    )


def supported_params(step: SupportedParamsStep) -> JsonValue:
    config = DashScopeChatConfig()
    return [*config.get_supported_openai_params(model=step["model"])]


def provider_info(step: ProviderInfoStep) -> JsonValue:
    set_env("DASHSCOPE_API_BASE", step["policy"].get("env_base"))
    set_env("DASHSCOPE_API_KEY", step.get("env_api_key"))
    _model, _provider, api_key, api_base = get_llm_provider(
        model=CHAT_MODEL,
        api_base=step.get("api_base"),
        api_key=step.get("api_key"),
    )
    return {"api_base": api_base, "api_key": api_key}


def cost(step: CostStep, index: int) -> JsonValue:
    model = f"{COST_MODEL_PREFIX}-{index}"
    pricing: dict[str, dict[str, JsonValue]] = {f"dashscope/{model}": step["model_info"]}
    litellm.register_model(pricing)
    moment = datetime.fromisoformat(step["current_time"])
    prompt_cost, completion_cost = cost_per_token(
        model=model,
        usage=build_usage(step["usage"]),
        custom_llm_provider="dashscope",
        current_time=moment,
    )
    return {
        "prompt_cost": format(Decimal(str(prompt_cost)).normalize(), "f"),
        "completion_cost": format(Decimal(str(completion_cost)).normalize(), "f"),
    }


def step_value(step: Step, index: int) -> JsonValue:
    if step["kind"] == "supported_params":
        return supported_params(step)
    if step["kind"] == "provider_info":
        return provider_info(step)
    return cost(step, index)


def run(input_value: Input) -> JsonValue:
    values: list[JsonValue] = []
    for index, step in enumerate(input_value["steps"]):
        values.append(step_value(step, index))
    return values
