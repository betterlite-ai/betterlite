import asyncio
import logging
from collections.abc import AsyncIterator
from typing import Final, Literal, TypedDict

from litellm.integrations.custom_guardrail import CustomGuardrail
from litellm.proxy._types import UserAPIKeyAuth
from litellm.types.guardrails import GuardrailEventHooks
from litellm.types.utils import ModelResponseStream
from pydantic import JsonValue as Json
from typing_extensions import override

from .chunks import CHUNK, ChunkInput, Hooks


class FlagCall(TypedDict):
    function: Literal["streaming_flag"]
    attribute_present: bool
    attribute: Json
    guardrail_config: dict[str, Json] | None
    optional_params: dict[str, Json]
    name: str
    default: Json


def streaming_flag(call: FlagCall) -> Json:
    guardrail = CustomGuardrail(guardrail_name="b3679")
    object.__setattr__(guardrail, "guardrail_config", call["guardrail_config"])
    if call["attribute_present"]:
        object.__setattr__(guardrail, call["name"], call["attribute"])
    hooks = Hooks(**call["optional_params"])
    return {"value": hooks.streaming_flag(guardrail, call["name"], call["default"])}


class ScanKeyInput(TypedDict):
    texts: list[str]
    tool_calls: list[str]
    stream_ended: bool
    tool_calls_in_flight: bool


class RedundantScanCall(TypedDict):
    function: Literal["redundant_scan"]
    chunks: list[ChunkInput]
    scan_keys: list[ScanKeyInput | None]


SCAN_LINE: Final = "Processing streaming chunk"


class ScanCounter(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.DEBUG)
        self.scans = 0

    @override
    def emit(self, record: logging.LogRecord) -> None:
        if record.getMessage().startswith(SCAN_LINE):
            self.scans += 1


async def source(chunks: list[ModelResponseStream]) -> AsyncIterator[ModelResponseStream]:
    for chunk in chunks:
        yield chunk


async def drain(chunks: list[ModelResponseStream]) -> None:
    sampled = Hooks(streaming_sampling_rate=1)
    guardrail = CustomGuardrail(
        guardrail_name="b3679", event_hook=GuardrailEventHooks.post_call, default_on=True
    )
    auth = UserAPIKeyAuth(request_route="/chat/completions")
    request_data: dict[str, Json] = {}
    async for _item in sampled.async_post_call_streaming_iterator_hook(  # type: ignore[misc]  # pyright: ignore[reportAny, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/unified_guardrail/unified_guardrail.py#L971-L978
        auth, source(chunks), request_data, guardrail
    ):
        pass


def redundant(call: RedundantScanCall) -> Json:
    logger = logging.getLogger("LiteLLM Proxy")
    counter = ScanCounter()
    level = logger.level
    handlers = logger.handlers[:]
    logger.handlers = [counter]
    logger.setLevel(logging.DEBUG)
    try:
        asyncio.run(drain([CHUNK.validate_python(chunk) for chunk in call["chunks"]]))
    finally:
        logger.handlers = handlers
        logger.setLevel(level)
    return {"scans": counter.scans}
