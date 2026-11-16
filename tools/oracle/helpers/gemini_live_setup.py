import json
import logging
import os
from logging.handlers import MemoryHandler
from typing import Literal, NotRequired, TypeAlias, TypedDict

import litellm
from fastuuid import UUID
from litellm._uuid import uuid as litellm_uuid
from litellm.llms.gemini.realtime.transformation import GeminiRealtimeConfig
from pydantic import JsonValue, TypeAdapter

JsonObject: TypeAlias = dict[str, JsonValue]

DEFAULT_EVENT_ID = "00000000-0000-4000-8000-000000000000"
STOCK_VOICE = "is an OpenAI voice with no Gemini equivalent"
DROPPED_UPDATE = "rejects a second setup message"


class Call(TypedDict):
    op: Literal["session_update", "automatic", "created", "endpoint"]
    model: str
    session: NotRequired[JsonObject]
    session_configuration_request: NotRequired[str | None]
    defer_setup: NotRequired[bool]
    logging_session_id: NotRequired[str]
    event_id: NotRequired[str]
    api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]
    env: NotRequired[dict[str, str]]


class Input(TypedDict):
    calls: list[Call]


INPUT = TypeAdapter(Input)
OBJECT = TypeAdapter(dict[str, JsonValue])
DOCUMENT = TypeAdapter(JsonValue)


class FixedUuid:
    def __init__(self, value: str) -> None:
        self.value = UUID(value)

    def uuid4(self) -> UUID:
        return self.value


def environment(call: Call) -> dict[str, str]:
    env = call.get("env")
    return {} if env is None else env


def session_payload(call: Call) -> JsonObject:
    session = call.get("session")
    return {} if session is None else session


def event_id(call: Call) -> str:
    value = call.get("event_id")
    return DEFAULT_EVENT_ID if value is None else value


def logging_session_id(call: Call) -> str:
    value = call.get("logging_session_id")
    return "" if value is None else value


def session_update(call: Call) -> JsonValue:
    config = GeminiRealtimeConfig()
    message = json.dumps({"type": "session.update", "session": session_payload(call)})
    messages = config.transform_realtime_request(
        message,
        call["model"],
        call.get("session_configuration_request"),
    )
    return {"messages": [DOCUMENT.validate_json(answer) for answer in messages]}


def automatic(call: Call) -> JsonValue:
    litellm.gemini_live_defer_setup = call.get("defer_setup", False)
    config = GeminiRealtimeConfig()
    required = config.requires_session_configuration()
    setup: JsonValue = None
    if required:
        envelope = OBJECT.validate_json(config.session_configuration_request(call["model"]))
        setup = envelope.get("setup")
    return {"required": required, "setup": setup}


def created(call: Call) -> JsonValue:
    litellm_uuid.uuid4 = FixedUuid(event_id(call)).uuid4
    config = GeminiRealtimeConfig()
    event = config.transform_session_created_event(
        call["model"],
        logging_session_id(call),
        call.get("session_configuration_request"),
    )
    return OBJECT.validate_python(event)


def endpoint(call: Call) -> JsonValue:
    config = GeminiRealtimeConfig()
    url = config.get_complete_url(
        call.get("api_base"),
        call["model"],
        call.get("api_key"),
    )
    return {"url": url}


def dispatch(call: Call) -> JsonValue:
    if call["op"] == "session_update":
        return session_update(call)
    if call["op"] == "automatic":
        return automatic(call)
    if call["op"] == "created":
        return created(call)
    return endpoint(call)


def label(message: str) -> str:
    if STOCK_VOICE in message:
        return "stock_voice_dropped"
    if DROPPED_UPDATE in message:
        return "later_update_dropped_with_guardrail"
    return message


def answer(call: Call) -> JsonValue:
    os.environ.clear()
    os.environ.update(environment(call))
    capture = MemoryHandler(capacity=1000, flushLevel=logging.CRITICAL)
    capture.setLevel(logging.WARNING)
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(capture)
    try:
        value = dispatch(call)
    finally:
        logger.removeHandler(capture)
    return {"value": value, "warnings": [label(record.getMessage()) for record in capture.buffer]}


def run(input_value: JsonValue) -> JsonValue:
    parsed = INPUT.validate_python(input_value)
    return {"calls": [answer(call) for call in parsed["calls"]]}
