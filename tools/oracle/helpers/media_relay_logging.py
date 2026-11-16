from datetime import UTC, datetime
from decimal import Decimal
from typing import TypedDict

import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.deepgram.common_utils import (
    deepgram_listen_audio_seconds,
    deepgram_listen_channel_count,
)
from litellm.proxy.pass_through_endpoints.llm_provider_handlers import (
    deepgram_listen_passthrough_logging_handler as deepgram_listen,
    fal_ai_passthrough_logging_handler as fal_passthrough,
)
from litellm.types.utils import ImageObject
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter


class FalRoute(TypedDict):
    url_route: str
    custom_llm_provider: str | None


class DeepgramCall(TypedDict):
    upstream_url: str
    messages: list[dict[str, JsonValue]]
    kwargs: dict[str, JsonValue]
    catalog: dict[str, dict[str, JsonValue]]


class FalCall(TypedDict):
    url_route: str
    response_body: dict[str, JsonValue]
    request_body: dict[str, JsonValue]
    kwargs: dict[str, JsonValue]
    catalog: dict[str, dict[str, JsonValue]]
    queue_base: str | None


class Input(TypedDict):
    deepgram_routes: list[str]
    fal_routes: list[FalRoute]
    deepgram_listens: list[DeepgramCall]
    fal_passthroughs: list[FalCall]


class Answer(TypedDict):
    deepgram_routes: list[bool]
    fal_routes: list[bool]
    deepgram_listens: list[JsonValue]
    fal_passthroughs: list[JsonValue]


class Details(BaseModel):
    model: str | None = None
    custom_llm_provider: str | None = None
    response_cost: float | None = None


class RecordedLogging(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    model_call_details: Details


class HandlerResult(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    text: str | None = None
    data: list[ImageObject] | None = None


class LoggedAnswer(BaseModel):
    result: HandlerResult
    kwargs: dict[str, JsonValue]


COST: TypeAdapter[float | None] = TypeAdapter(float | None)


def make_logging() -> Logging:
    return Logging(
        model="fixture-model",
        messages=None,
        stream=False,
        call_type="pass_through_endpoint",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
    )


def decimal_text(value: float | None) -> str | None:
    if value is None:
        return None
    return format(Decimal(repr(value)).normalize(), "f")


def kwargs_view(kwargs: dict[str, JsonValue]) -> dict[str, JsonValue]:
    view = dict(kwargs)
    view["response_cost"] = decimal_text(COST.validate_python(view.get("response_cost")))
    return view


def transcript_text(result: HandlerResult) -> str:
    text = result.text
    return "" if text is None else text


def image_urls(result: HandlerResult) -> list[JsonValue]:
    urls: list[JsonValue] = []
    if result.data is None:
        return urls
    for image in result.data:
        url = image.url
        if url is not None:
            urls.append(url)
    return urls


def deepgram_answer(call: DeepgramCall) -> JsonValue:
    litellm.model_cost = dict(call["catalog"])
    logging_obj = make_logging()
    handler = deepgram_listen.DeepgramListenPassthroughLoggingHandler()
    returned = LoggedAnswer.model_validate(
        handler.deepgram_listen_passthrough_handler(
            websocket_messages=call["messages"],
            logging_obj=logging_obj,
            upstream_url=call["upstream_url"],
            kwargs=call["kwargs"],
        )
    )
    details = RecordedLogging.model_validate(logging_obj).model_call_details
    audio_seconds = deepgram_listen_audio_seconds(call["messages"])
    channels = deepgram_listen_channel_count(call["messages"], call["upstream_url"])
    answer: dict[str, JsonValue] = {
        "text": transcript_text(returned.result),
        "billed_seconds": decimal_text(audio_seconds * channels),
        "model": details.model,
        "custom_llm_provider": details.custom_llm_provider,
        "response_cost": decimal_text(details.response_cost),
        "kwargs": kwargs_view(returned.kwargs),
    }
    return answer


def fal_answer(call: FalCall) -> JsonValue:
    litellm.model_cost = dict(call["catalog"])
    logging_obj = make_logging()
    handler = fal_passthrough.FalAIPassthroughLoggingHandler()
    returned = LoggedAnswer.model_validate(
        handler.fal_ai_passthrough_handler(
            response_body=call["response_body"],
            request_body=call["request_body"],
            logging_obj=logging_obj,
            url_route=call["url_route"],
            kwargs=call["kwargs"],
        )
    )
    details = RecordedLogging.model_validate(logging_obj).model_call_details
    answer: dict[str, JsonValue] = {
        "urls": image_urls(returned.result),
        "model": details.model,
        "custom_llm_provider": details.custom_llm_provider,
        "response_cost": decimal_text(details.response_cost),
        "kwargs": kwargs_view(returned.kwargs),
    }
    return answer


def run(input_value: Input) -> Answer:
    deepgram_routes = [
        deepgram_listen.DeepgramListenPassthroughLoggingHandler.is_deepgram_listen_route(route)
        for route in input_value["deepgram_routes"]
    ]
    fal_routes = [
        fal_passthrough.FalAIPassthroughLoggingHandler.is_fal_ai_route(
            call["url_route"], call["custom_llm_provider"]
        )
        for call in input_value["fal_routes"]
    ]
    answer: Answer = {
        "deepgram_routes": deepgram_routes,
        "fal_routes": fal_routes,
        "deepgram_listens": [deepgram_answer(call) for call in input_value["deepgram_listens"]],
        "fal_passthroughs": [fal_answer(call) for call in input_value["fal_passthroughs"]],
    }
    return answer
