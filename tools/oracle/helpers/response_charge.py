from datetime import UTC, datetime
from typing import Literal, NotRequired, TypedDict

import httpx
import litellm
from litellm import cost_calculator
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.ocr.transformation import OCRResponse, OCRUsageInfo
from litellm.types.utils import (
    CostPerToken,
    ImageResponse,
    LiteLLMRealtimeStreamLoggingObject,
    ModelResponse,
    StandardBuiltInToolsParams,
    TranscriptionResponse,
    TranscriptionUsageInputTokenDetailsObject,
    TranscriptionUsageTokensObject,
    Usage,
)
from pydantic import JsonValue

CallKind = Literal[
    "completion",
    "image_generation",
    "transcription",
    "ocr",
    "search",
    "send_message",
    "call_mcp_tool",
    "_arealtime",
    "_aresponses_websocket",
]
ResponseKind = Literal["chat", "image", "transcription", "ocr", "realtime_stream", "none"]


class Entry(TypedDict):
    litellm_provider: str
    mode: str
    input_cost_per_token: float
    output_cost_per_token: float
    cache_read_input_token_cost: NotRequired[float | None]
    input_cost_per_image: NotRequired[float]
    input_cost_per_query: NotRequired[float]
    ocr_cost_per_credit: NotRequired[float]
    ocr_cost_per_page: NotRequired[float]
    output_cost_per_second: NotRequired[float]
    input_cost_per_token_priority: NotRequired[float]
    output_cost_per_token_priority: NotRequired[float]
    input_cost_per_token_flex: NotRequired[float]
    output_cost_per_token_flex: NotRequired[float]
    search_context_cost_per_query: NotRequired[dict[str, float]]


class UsageInput(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    cached_tokens: int | None


class Rates(TypedDict):
    input_cost_per_token: float
    output_cost_per_token: float


class Split(TypedDict):
    percentage: float | None
    fixed_amount: float | None


class OcrInput(TypedDict):
    credits: float | None
    pages_processed: int | None
    pages_processed_annotation: int | None


class AgentInput(TypedDict):
    response_cost: float | None
    cost_per_query: float | None


class Input(TypedDict):
    function: Literal["completion_cost", "response_cost_calculator"]
    model: str
    custom_llm_provider: str | None
    region_name: str | None
    catalog: dict[str, Entry]
    usage: UsageInput
    custom_price: Rates | None
    cache_hit: bool | None
    header_cost: float | None
    discount: dict[str, float]
    margin: dict[str, Split]
    call_type: NotRequired[CallKind | None]
    response_kind: NotRequired[ResponseKind]
    response_model: NotRequired[str | None]
    optional_params: NotRequired[dict[str, JsonValue] | None]
    standard_tools: NotRequired[StandardBuiltInToolsParams | None]
    annotations: NotRequired[bool | None]
    logging_model: NotRequired[str | None]
    agent: NotRequired[AgentInput | None]
    refuse_breakdown: NotRequired[bool | None]
    custom_pricing: NotRequired[bool | None]
    service_tier: NotRequired[str | None]
    transcription_seconds: NotRequired[float | None]
    ocr: NotRequired[OcrInput | None]
    events: NotRequired[list[dict[str, JsonValue]] | None]


class Output(TypedDict):
    cost: float


WrapperResponse = (
    ModelResponse
    | ImageResponse
    | TranscriptionResponse
    | LiteLLMRealtimeStreamLoggingObject
)

CompletionResponse = WrapperResponse | OCRResponse | None


def margin_config(margin: dict[str, Split]) -> dict[str, float | dict[str, float]]:
    config: dict[str, float | dict[str, float]] = {}
    for provider, entry in margin.items():
        fields: dict[str, float] = {}
        percentage = entry["percentage"]
        if percentage is not None:
            fields["percentage"] = percentage
        fixed_amount = entry["fixed_amount"]
        if fixed_amount is not None:
            fields["fixed_amount"] = fixed_amount
        config[provider] = fields
    return config


def usage_value(fields: UsageInput) -> Usage:
    values: dict[str, JsonValue] = {
        "prompt_tokens": fields["prompt_tokens"],
        "completion_tokens": fields["completion_tokens"],
        "total_tokens": fields["prompt_tokens"] + fields["completion_tokens"],
    }
    cached = fields["cached_tokens"]
    if cached is not None:
        details: dict[str, JsonValue] = {"cached_tokens": cached}
        values["prompt_tokens_details"] = details
    return Usage.model_validate(values)


def usage_payload(fields: UsageInput) -> dict[str, JsonValue]:
    payload: dict[str, JsonValue] = {
        "prompt_tokens": fields["prompt_tokens"],
        "completion_tokens": fields["completion_tokens"],
        "total_tokens": fields["prompt_tokens"] + fields["completion_tokens"],
    }
    cached = fields["cached_tokens"]
    if cached is not None:
        payload["prompt_tokens_details"] = {"cached_tokens": cached}
    return payload


def response_kind(input_value: Input) -> ResponseKind:
    kind = input_value.get("response_kind")
    if kind is None:
        return "chat"
    return kind


def chat_response(
    input_value: Input, usage: Usage, payload: dict[str, JsonValue]
) -> ModelResponse:
    model = input_value.get("response_model")
    if input_value.get("annotations"):
        response = ModelResponse.model_validate(
            {
                "id": "response-charge-case",
                "created": 0,
                "object": "chat.completion",
                "model": model,
                "usage": payload,
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": "",
                            "annotations": [
                                {
                                    "type": "url_citation",
                                    "url_citation": {
                                        "url": "https://example.invalid",
                                        "title": "case",
                                        "start_index": 0,
                                        "end_index": 0,
                                    },
                                }
                            ],
                        }
                    }
                ],
            }
        )
    else:
        response = ModelResponse(model=model, usage=usage)
    header = input_value["header_cost"]
    if header is not None:
        response.set_provider_response_headers(
            httpx.Headers({"x-litellm-response-cost": str(header)})
        )
    return response


def image_response(input_value: Input) -> ImageResponse:
    return ImageResponse.model_validate(
        {
            "created": 0,
            "data": [{"url": "https://example.invalid/image.png"}],
            "model": input_value.get("response_model"),
        }
    )


def transcription_response(input_value: Input) -> TranscriptionResponse:
    fields = input_value["usage"]
    response = TranscriptionResponse(text="")
    seconds = input_value.get("transcription_seconds")
    if seconds is not None:
        response["duration"] = seconds
    response.usage = TranscriptionUsageTokensObject(
        type="tokens",
        input_tokens=fields["prompt_tokens"],
        output_tokens=fields["completion_tokens"],
        total_tokens=fields["prompt_tokens"] + fields["completion_tokens"],
        input_token_details=TranscriptionUsageInputTokenDetailsObject(
            audio_tokens=0, text_tokens=fields["prompt_tokens"]
        ),
    )
    return response


def ocr_response(input_value: Input) -> OCRResponse:
    fields = input_value.get("ocr")
    if fields is None:
        raise ValueError("an OCR response needs its usage")
    usage = OCRUsageInfo(
        credits=fields["credits"],
        pages_processed=fields["pages_processed"],
        pages_processed_annotation=fields["pages_processed_annotation"],
    )
    return OCRResponse(pages=[], model=input_value["model"], usage_info=usage)


def realtime_response(
    input_value: Input, usage: Usage
) -> LiteLLMRealtimeStreamLoggingObject:
    events = input_value.get("events")
    if events is None:
        raise ValueError("a realtime stream response needs its events")
    return LiteLLMRealtimeStreamLoggingObject.model_validate(
        {"results": events, "usage": usage}
    )


def wrapper_response(
    input_value: Input, usage: Usage, payload: dict[str, JsonValue]
) -> WrapperResponse:
    kind = response_kind(input_value)
    if kind == "chat":
        return chat_response(input_value, usage, payload)
    if kind == "image":
        return image_response(input_value)
    if kind == "transcription":
        return transcription_response(input_value)
    if kind == "realtime_stream":
        return realtime_response(input_value, usage)
    raise ValueError("the response wrapper needs a priced response")


def completion_response(
    input_value: Input, usage: Usage, payload: dict[str, JsonValue]
) -> CompletionResponse:
    kind = response_kind(input_value)
    if kind == "ocr":
        return ocr_response(input_value)
    if kind == "none":
        return None
    return wrapper_response(input_value, usage, payload)


def agent_details(agent: AgentInput) -> dict[str, JsonValue]:
    params: dict[str, JsonValue] = {"cost_per_query": agent["cost_per_query"]}
    details: dict[str, JsonValue] = {"litellm_params": params}
    response_cost = agent["response_cost"]
    if response_cost is not None:
        details["response_cost"] = response_cost
    return details


def build_logging(input_value: Input) -> Logging | None:
    model = input_value.get("logging_model")
    agent = input_value.get("agent")
    if model is None and agent is None:
        return None
    log = Logging(
        model=model,
        messages=None,
        stream=False,
        call_type="completion",
        start_time=datetime(2024, 1, 1, tzinfo=UTC),
        litellm_call_id="response-charge-case",
        function_id="response-charge-case",
    )
    if agent is not None:
        log.model_call_details = agent_details(agent)
    return log


def call_cost(input_value: Input, log: Logging | None) -> float:
    prices = input_value["custom_price"]
    rates: CostPerToken | None = None
    if prices is not None:
        rates = CostPerToken(
            input_cost_per_token=prices["input_cost_per_token"],
            output_cost_per_token=prices["output_cost_per_token"],
        )
    usage = usage_value(input_value["usage"])
    payload = usage_payload(input_value["usage"])
    if input_value["function"] == "completion_cost":
        return litellm.completion_cost(
            completion_response=completion_response(input_value, usage, payload),
            model=input_value["model"],
            call_type=input_value.get("call_type"),
            custom_llm_provider=input_value["custom_llm_provider"],
            region_name=input_value["region_name"],
            custom_cost_per_token=rates,
            optional_params=input_value.get("optional_params"),
            standard_built_in_tools_params=input_value.get("standard_tools"),
            litellm_logging_obj=log,
            custom_pricing=input_value.get("custom_pricing"),
            service_tier=input_value.get("service_tier"),
        )
    return cost_calculator.response_cost_calculator(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/cost_calculator.py#L1979
        response_object=wrapper_response(input_value, usage, payload),
        model=input_value["model"],
        custom_llm_provider=input_value["custom_llm_provider"],
        call_type="completion",
        optional_params={},
        cache_hit=input_value["cache_hit"],
        region_name=input_value["region_name"],
    )


def run(input_value: Input) -> Output:
    for model, entry in input_value["catalog"].items():
        record = {key: value for key, value in entry.items() if value is not None}
        litellm.register_model({model: record})
    litellm.cost_discount_config = input_value["discount"]
    litellm.cost_margin_config = margin_config(input_value["margin"])
    log = build_logging(input_value)
    return {"cost": call_cost(input_value, log)}
