from datetime import datetime
from decimal import Decimal
from typing import Literal, NotRequired, TypeAlias, TypedDict, TypeVar, cast

import litellm
import litellm.llms.vertex_ai.cost_calculator as vertex_cost
from litellm.llms.gemini import cost_calculator as studio_cost
from litellm.llms.vertex_ai.gemini import cost_calculator as vertex_gemini_cost
from litellm.types.utils import (
    ModelInfo,
    PromptTokensDetailsWrapper,
    ServerToolUse,
    Usage,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
Value = TypeVar("Value")


class DetailsInput(TypedDict, total=False):
    web_search_requests: int | None
    google_maps_grounding_requests: int | None


class ServerToolUseInput(TypedDict, total=False):
    web_search_requests: int | None


class UsageInput(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    prompt_tokens_details: NotRequired[DetailsInput | None]
    server_tool_use: NotRequired[ServerToolUseInput | None]


class Input(TypedDict):
    operation: Literal[
        "cost_router",
        "cost_per_character",
        "cost_per_token",
        "web_search_cost",
        "maps_grounding_cost",
    ]
    model: NotRequired[str]
    custom_llm_provider: NotRequired[str]
    call_type: NotRequired[str]
    record: NotRequired[Object]
    usage: NotRequired[UsageInput]
    prompt_characters: NotRequired[float | None]
    completion_characters: NotRequired[float | None]
    service_tier: NotRequired[str | None]
    vertex_location: NotRequired[str | None]
    entry: NotRequired[Literal["gemini", "vertex_gemini"]]


class UnexpectedCallError(Exception):
    pass


RECORDED_CALLS: list[Json] = []


def money(value: float) -> str:
    exact = Decimal(repr(float(value))).normalize()
    return format(exact, "f")


def required(value: Value | None, name: str) -> Value:
    if value is None:
        raise ValueError(f"{name} is required")
    return value


def record_info(input_value: Input) -> ModelInfo:
    record = required(input_value.get("record"), "record")
    return cast(ModelInfo, record)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L426


class FixtureCatalog:
    def __init__(self, model: str, provider: str, record: Object) -> None:
        self.model = model
        self.provider = provider
        self.record = record

    def __call__(
        self,
        model: str,
        custom_llm_provider: str | None = None,
        api_base: str | None = None,
        api_key: str | None = None,
    ) -> ModelInfo:
        if (model, custom_llm_provider) != (self.model, self.provider):
            raise UnexpectedCallError(model, custom_llm_provider)
        if api_base is not None or api_key is not None:
            raise UnexpectedCallError(api_base, api_key)
        return cast(ModelInfo, self.record)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L426


def recorded_generic_cost(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/llm_cost_calc/utils.py#L1291
    model: str,
    usage: Usage,
    custom_llm_provider: str,
    service_tier: str | None = None,
    data_residency: str | None = None,
    model_info: ModelInfo | None = None,
    vertex_location: str | None = None,
    current_time: datetime | None = None,
) -> tuple[float, float]:
    del data_residency, model_info, current_time  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L426
    recorded: Json = {
        "model": model,
        "provider": custom_llm_provider,
        "service_tier": service_tier,
        "vertex_location": vertex_location,
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
    }
    RECORDED_CALLS.append(recorded)
    return float(usage.prompt_tokens), float(usage.completion_tokens)


def usage_object(value: UsageInput) -> Usage:
    raw_details = value.get("prompt_tokens_details")
    prompt_details = None
    if raw_details is not None:
        prompt_details = PromptTokensDetailsWrapper.model_construct(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1748
            web_search_requests=raw_details.get("web_search_requests"),
            google_maps_grounding_requests=raw_details.get(
                "google_maps_grounding_requests"
            ),
        )
    raw_server = value.get("server_tool_use")
    server_tool_use = None
    if raw_server is not None:
        server_tool_use = ServerToolUse.model_construct(
            web_search_requests=raw_server.get("web_search_requests")
        )
    prompt_tokens = value["prompt_tokens"]
    completion_tokens = value["completion_tokens"]
    return Usage.model_construct(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1857
        prompt_tokens=prompt_tokens,
        completion_tokens=completion_tokens,
        total_tokens=prompt_tokens + completion_tokens,
        prompt_tokens_details=prompt_details,
        completion_tokens_details=None,
        server_tool_use=server_tool_use,
    )


def install_catalog(input_value: Input) -> None:
    model = required(input_value.get("model"), "model")
    provider = required(input_value.get("custom_llm_provider"), "custom_llm_provider")
    record = required(input_value.get("record"), "record")
    litellm.get_model_info = FixtureCatalog(model, provider, record)
    vertex_cost.generic_cost_per_token = recorded_generic_cost  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/llm_cost_calc/utils.py#L1291


def cost_record(prompt_cost: float, completion_cost: float) -> Json:
    return {
        "prompt_cost": money(prompt_cost),
        "completion_cost": money(completion_cost),
        "calls": RECORDED_CALLS,
    }


def router_result(input_value: Input) -> Json:
    model = required(input_value.get("model"), "model")
    provider = required(input_value.get("custom_llm_provider"), "custom_llm_provider")
    call_type = required(input_value.get("call_type"), "call_type")
    return {"metric": vertex_cost.cost_router(model, provider, call_type)}


def cost_result(
    input_value: Input, operation: Literal["cost_per_character", "cost_per_token"]
) -> Json:
    install_catalog(input_value)
    usage = usage_object(required(input_value.get("usage"), "usage"))
    model = required(input_value.get("model"), "model")
    provider = required(input_value.get("custom_llm_provider"), "custom_llm_provider")
    if operation == "cost_per_character":
        prompt_cost, completion_cost = vertex_cost.cost_per_character(
            model=model,
            custom_llm_provider=provider,
            usage=usage,
            prompt_characters=input_value.get("prompt_characters"),
            completion_characters=input_value.get("completion_characters"),
            service_tier=input_value.get("service_tier"),
            vertex_location=input_value.get("vertex_location"),
        )
    else:
        prompt_cost, completion_cost = vertex_cost.cost_per_token(
            model=model,
            custom_llm_provider=provider,
            usage=usage,
            service_tier=input_value.get("service_tier"),
            vertex_location=input_value.get("vertex_location"),
        )
    return cost_record(prompt_cost, completion_cost)


def search_result(input_value: Input) -> Json:
    usage = usage_object(required(input_value.get("usage"), "usage"))
    record = record_info(input_value)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L426
    if input_value.get("entry") == "vertex_gemini":
        cost = vertex_gemini_cost.cost_per_web_search_request(
            usage, record  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/gemini/cost_calculator.py#L14
        )
    else:
        cost = studio_cost.cost_per_web_search_request(
            usage, record  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/cost_calculator.py#L29
        )
    return {"cost": money(cost)}


def maps_result(input_value: Input) -> Json:
    usage = usage_object(required(input_value.get("usage"), "usage"))
    cost = studio_cost.cost_per_google_maps_grounding_request(
        usage, record_info(input_value)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/cost_calculator.py#L86
    )
    return {"cost": money(cost)}


def run(input_value: Input) -> Json:
    RECORDED_CALLS.clear()
    match input_value["operation"]:
        case "cost_router":
            return router_result(input_value)
        case "cost_per_character":
            return cost_result(input_value, "cost_per_character")
        case "cost_per_token":
            return cost_result(input_value, "cost_per_token")
        case "web_search_cost":
            return search_result(input_value)
        case "maps_grounding_cost":
            return maps_result(input_value)
