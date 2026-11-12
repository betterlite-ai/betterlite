from decimal import Decimal
from typing import Literal, TypeAlias, TypedDict, cast

from litellm.litellm_core_utils.llm_cost_calc.utils import get_web_search_requests
from litellm.llms.anthropic.cost_calculation import (
    cost_per_token,
    get_anthropic_web_search_requests_from_response,
    get_cost_for_anthropic_web_search,
)
from litellm.types.utils import ModelInfo, ServerToolUse, Usage

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class UsageInput(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class ServerToolUseInput(TypedDict, total=False):
    web_search_requests: int | None
    tool_search_requests: int | None
    browser_open_requests: int | None


class MappingRecord(TypedDict):
    kind: Literal["mapping"]
    value: dict[str, Json]


class AttributeRecord(TypedDict):
    kind: Literal["attribute"]
    value: ServerToolUseInput


Record: TypeAlias = MappingRecord | AttributeRecord


class TokenCall(TypedDict):
    kind: Literal["token"]
    model: str
    usage: UsageInput
    model_info: dict[str, Json] | None
    provider_specific_entry: dict[str, Json] | None
    service_tier: str | None
    inference_geo: str | None
    speed: str | None


class RawProbeCall(TypedDict):
    kind: Literal["raw_probe"]
    responses: list[Json]


class AccessorCall(TypedDict):
    kind: Literal["web_search_requests"]
    record: Record | None


class SearchCall(TypedDict):
    kind: Literal["search_cost"]
    model_info: dict[str, Json] | None
    search_context: dict[str, Json] | None
    record: Record | None


Input: TypeAlias = TokenCall | RawProbeCall | AccessorCall | SearchCall


def decimal_text(value: float) -> str:
    if not value:
        return "0"
    return format(Decimal(repr(value)).normalize(), "f")


def attribute_record(value: ServerToolUseInput) -> ServerToolUse:
    return ServerToolUse(
        web_search_requests=value.get("web_search_requests"),
        tool_search_requests=value.get("tool_search_requests"),
        browser_open_requests=value.get("browser_open_requests"),
    )


def typed_model_info(
    model_info: dict[str, Json] | None,
    entry: dict[str, Json] | None,
    nested_key: str,
) -> ModelInfo | None:
    if model_info is None:
        return None
    info = dict(model_info)
    if entry is not None:
        info[nested_key] = entry
    return cast("ModelInfo", info)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L402


def search_usage(record: Record | None) -> Usage | None:
    if record is None:
        return None
    usage = Usage()
    if record["kind"] == "mapping":
        usage.server_tool_use = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/llm_cost_calc/utils.py#L126
            "ServerToolUse", record["value"]
        )
    else:
        usage.server_tool_use = attribute_record(record["value"])
    return usage


def token_value(call: TokenCall) -> Json:
    usage = Usage(
        prompt_tokens=call["usage"]["prompt_tokens"],
        completion_tokens=call["usage"]["completion_tokens"],
        total_tokens=call["usage"]["total_tokens"],
        inference_geo=call["inference_geo"],
        speed=call["speed"],
    )
    prompt_cost, completion_cost = cost_per_token(
        model=call["model"],
        usage=usage,
        service_tier=call["service_tier"],
        model_info=typed_model_info(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L402
            call["model_info"], call["provider_specific_entry"], "provider_specific_entry"
        ),
    )
    return {
        "prompt_cost": decimal_text(prompt_cost),
        "completion_cost": decimal_text(completion_cost),
    }


def raw_probe_value(call: RawProbeCall) -> Json:
    values: list[Json] = [
        get_anthropic_web_search_requests_from_response(response) for response in call["responses"]
    ]
    return values


def accessor_value(call: AccessorCall) -> Json:
    record = call["record"]
    if record is None:
        return get_web_search_requests(None)
    if record["kind"] == "mapping":
        return get_web_search_requests(record["value"])
    return get_web_search_requests(attribute_record(record["value"]))


def search_value(call: SearchCall) -> Json:
    cost = get_cost_for_anthropic_web_search(
        model_info=typed_model_info(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L402
            call["model_info"], call["search_context"], "search_context_cost_per_query"
        ),
        usage=search_usage(call["record"]),
    )
    return {"cost": decimal_text(cost)}


def run(input_value: Input) -> Json:
    if input_value["kind"] == "token":
        return token_value(input_value)
    if input_value["kind"] == "raw_probe":
        return raw_probe_value(input_value)
    if input_value["kind"] == "web_search_requests":
        return accessor_value(input_value)
    return search_value(input_value)
