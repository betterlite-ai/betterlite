import asyncio
import json
import sys
from collections.abc import AsyncIterator
from datetime import datetime
from types import ModuleType
from typing import Protocol, TypedDict, Unpack

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.anthropic.pass_through.messages.transformation import (
    AnthropicMessagesConfig,
)
from litellm.proxy.pass_through_endpoints.success_handler import PassThroughEndpointLogging
from litellm.types.passthrough_endpoints.pass_through_endpoints import EndpointType
from pydantic import BaseModel, ConfigDict, JsonValue

MODULE = "litellm.proxy.pass_through_endpoints.streaming_handler"
TIMEOUT_SECONDS = 120.0


class SuppliedLogging(Logging):
    def __init__(self) -> None:
        details: dict[str, JsonValue] = {}
        self.model_call_details = details
        self.details = details


class FactoryCall(TypedDict):
    model: str
    body: dict[str, JsonValue]


class Segment(TypedDict):
    response: httpx.Response
    request_body: dict[str, JsonValue] | None
    litellm_logging_obj: Logging
    endpoint_type: EndpointType
    start_time: datetime
    passthrough_success_handler_obj: PassThroughEndpointLogging  # ast-grep-ignore: no-long-names
    url_route: str


class Recorder:
    def __init__(self) -> None:
        self.seen: Segment | None = None

    async def chunk_processor(self, **call: Unpack[Segment]) -> AsyncIterator[bytes]:
        self.seen = call
        async for chunk in call["response"].aiter_bytes():
            yield chunk


class HandlerModule(ModuleType):
    PassThroughStreamingHandler: Recorder


def install(recorder: Recorder) -> None:
    module = HandlerModule(MODULE)
    module.PassThroughStreamingHandler = recorder
    sys.modules[MODULE] = module


class ConfigFactory(Protocol):
    def get_async_streaming_response_iterator(  # ast-grep-ignore: no-long-names
        self,
        model: str,
        httpx_response: httpx.Response,
        request_body: dict[str, JsonValue],
        litellm_logging_obj: Logging,
    ) -> AsyncIterator[bytes]: ...


def config_iterator(
    config: ConfigFactory, call: FactoryCall, reply: httpx.Response, logging_obj: Logging
) -> AsyncIterator[bytes]:
    return config.get_async_streaming_response_iterator(
        model=call["model"],
        httpx_response=reply,
        request_body=call["body"],
        litellm_logging_obj=logging_obj,
    )


def frame_list(text: str) -> list[JsonValue]:
    frames: list[JsonValue] = []
    for frame in text.split("\n\n"):
        if frame:
            frames.append(frame)
    return frames


def observed(
    recorder: Recorder, reply: httpx.Response, text: str, supplied: Logging
) -> JsonValue:
    call = recorder.seen
    if call is None:
        raise ValueError("the factory never reached the chunk processor")
    return {
        "frames": frame_list(text),
        "closed": reply.is_closed,
        "request_body": call["request_body"],
        "logging_object_passed": call["litellm_logging_obj"] is supplied,
        "endpoint_anthropic": call["endpoint_type"] == EndpointType.ANTHROPIC,
        "url_route": call["url_route"],
        "response_status": call["response"].status_code,
    }


async def collect(call: FactoryCall, api_base: str) -> JsonValue:
    supplied = SuppliedLogging()
    recorder = Recorder()
    install(recorder)
    async with (
        httpx.AsyncClient(trust_env=False, timeout=TIMEOUT_SECONDS) as client,
        client.stream(
            "POST",
            f"{api_base}/messages",
            json=call["body"],
            headers={"content-type": "application/json"},
        ) as reply,
    ):
        iterator = config_iterator(AnthropicMessagesConfig(), call, reply, supplied)
        payload = bytearray()
        async for chunk in iterator:
            payload.extend(chunk)
        return observed(recorder, reply, payload.decode(errors="replace"), supplied)


class MessagesStreamFactoryCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model: str
    body: dict[str, JsonValue]


def run_messages_stream_factory(call: MessagesStreamFactoryCall, api_base: str) -> str:
    request: FactoryCall = {"model": call.model, "body": call.body}
    value = asyncio.run(collect(request, api_base))
    outcome: JsonValue = {"messages_stream_factory": {"value": value}}
    return json.dumps(outcome)
