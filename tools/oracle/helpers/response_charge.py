import json
from datetime import UTC, datetime
from typing import Literal, NotRequired, TypedDict

import httpx
import litellm
from litellm import cost_calculator
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.ocr.transformation import OCRResponse, OCRUsageInfo
from litellm.types.llms.openai import ChatCompletionAnnotation
from litellm.types.utils import (
    Choices,
    CostPerToken,
    ImageResponse,
    Message,
    ModelResponse,
    PromptTokensDetailsWrapper,
    StandardBuiltInToolsParams,
    TranscriptionResponse,
    TranscriptionUsageInputTokenDetailsObject,
    TranscriptionUsageTokensObject,
    Usage,
)
from litellm.types.utils import LiteLLMRealtimeStreamLoggingObject as RealtimeLogging
from pydantic import JsonValue, TypeAdapter

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
    catalog: dict[str, dict[str, JsonValue]]
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
    events: NotRequired[list[JsonValue] | None]


WrapperResponse = (
    ModelResponse | ImageResponse | TranscriptionResponse | RealtimeLogging
)

CompletionResponse = WrapperResponse | OCRResponse | None


def usage_value(fields: UsageInput) -> Usage:
    prompt = fields["prompt_tokens"]
    completion = fields["completion_tokens"]
    cached = fields["cached_tokens"]
    details = None if cached is None else PromptTokensDetailsWrapper(cached_tokens=cached)
    return Usage(
        prompt_tokens=prompt, completion_tokens=completion, total_tokens=prompt + completion,
        prompt_tokens_details=details,
    )


def response_kind(input_value: Input) -> ResponseKind:
    kind = input_value.get("response_kind")
    if kind is None:
        return "chat"
    return kind


def chat_response(input_value: Input, usage: Usage) -> ModelResponse:
    model = input_value.get("response_model")
    choices: list[Choices] | None = None
    if input_value.get("annotations"):
        citation: ChatCompletionAnnotation = {
            "type": "url_citation",
            "url_citation": {
                "url": "https://example.invalid",
                "title": "case",
                "start_index": 0,
                "end_index": 0,
            },
        }
        choices = [Choices(message=Message(content="", role="assistant", annotations=[citation]))]
    response = ModelResponse(model=model, usage=usage, choices=choices)
    header = input_value["header_cost"]
    if header is not None:
        response.set_provider_response_headers(
            httpx.Headers({"x-litellm-response-cost": str(header)})
        )
    return response


def image_response(input_value: Input) -> ImageResponse:
    document: dict[str, JsonValue] = {
        "created": 0, "model": input_value.get("response_model"),
        "data": [{"url": "https://example.invalid/image.png"}],
    }
    return TypeAdapter(ImageResponse).validate_json(json.dumps(document))


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


def realtime_response(input_value: Input) -> RealtimeLogging:
    events = input_value.get("events")
    if events is None:
        raise ValueError("a realtime stream response needs its events")
    fields = input_value["usage"]
    prompt = fields["prompt_tokens"]
    completion = fields["completion_tokens"]
    usage: dict[str, JsonValue] = {
        "prompt_tokens": prompt,
        "completion_tokens": completion,
        "total_tokens": prompt + completion,
    }
    document: dict[str, JsonValue] = {"results": events, "usage": usage}
    return TypeAdapter(RealtimeLogging).validate_json(json.dumps(document))


def wrapper_response(input_value: Input, usage: Usage) -> WrapperResponse:
    kind = response_kind(input_value)
    if kind == "chat":
        return chat_response(input_value, usage)
    if kind == "image":
        return image_response(input_value)
    if kind == "transcription":
        return transcription_response(input_value)
    if kind == "realtime_stream":
        return realtime_response(input_value)
    raise ValueError("the response wrapper needs a priced response")


def completion_response(input_value: Input, usage: Usage) -> CompletionResponse:
    kind = response_kind(input_value)
    if kind == "ocr":
        return ocr_response(input_value)
    if kind == "none":
        return None
    return wrapper_response(input_value, usage)


def build_logging(input_value: Input) -> Logging | None:
    model = input_value.get("logging_model")
    agent = input_value.get("agent")
    if model is None and agent is None:
        return None
    log = Logging(
        model=model if model is not None else input_value["model"],
        messages=None,
        stream=False,
        call_type="completion",
        start_time=datetime(2024, 1, 1, tzinfo=UTC),
        litellm_call_id="response-charge-case",
        function_id="response-charge-case",
    )
    if agent is not None:
        params: dict[str, JsonValue] = {"cost_per_query": agent["cost_per_query"]}
        details: dict[str, JsonValue] = {"litellm_params": params}
        response_cost = agent["response_cost"]
        if response_cost is not None:
            details["response_cost"] = response_cost
        log.model_call_details = details
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
    if input_value["function"] == "completion_cost":
        return litellm.completion_cost(
            completion_response=completion_response(input_value, usage),
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
    params: dict[str, JsonValue] = {}
    return cost_calculator.response_cost_calculator(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/cost_calculator.py#L1979
        response_object=wrapper_response(input_value, usage),
        model=input_value["model"],
        custom_llm_provider=input_value["custom_llm_provider"],
        call_type="completion",
        optional_params=params,
        cache_hit=input_value["cache_hit"],
        region_name=input_value["region_name"],
    )


def run(input_value: Input) -> dict[str, float]:
    models: dict[str, dict[str, JsonValue]] = {}
    for model, entry in input_value["catalog"].items():
        models[model] = {key: value for key, value in entry.items() if value is not None}
    litellm.register_model(models)
    litellm.cost_discount_config = input_value["discount"]
    margin: dict[str, float | dict[str, float]] = {}
    for provider, split in input_value["margin"].items():
        fields: dict[str, float] = {}
        percentage = split["percentage"]
        if percentage is not None:
            fields["percentage"] = percentage
        fixed_amount = split["fixed_amount"]
        if fixed_amount is not None:
            fields["fixed_amount"] = fixed_amount
        margin[provider] = fields
    litellm.cost_margin_config = margin
    log = build_logging(input_value)
    return {"cost": call_cost(input_value, log)}
