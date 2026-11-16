import json
import logging
import os
from logging.handlers import MemoryHandler
from typing import Final, Literal, NotRequired, Protocol, TypedDict, runtime_checkable

import litellm
from litellm.llms.vertex_ai.realtime.transformation import VertexAIRealtimeConfig
from pydantic import ConfigDict, JsonValue, TypeAdapter

litellm.suppress_debug_info = True

DROPPED_UPDATE = "rejects a second setup message"


class Session(TypedDict):
    project: str
    location: str
    access_token: str


class Call(TypedDict):
    op: Literal["endpoint", "mime", "automatic", "transform"]
    session: Session
    api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]
    headers: NotRequired[dict[str, str]]
    formats: NotRequired[list[str]]
    model: NotRequired[str]
    message: NotRequired[JsonValue]
    session_configuration_request: NotRequired[str | None]
    env: NotRequired[dict[str, str]]


class EndpointCall(TypedDict):
    session: Session
    api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]
    headers: NotRequired[dict[str, str]]


class MimeCall(TypedDict):
    session: Session
    formats: list[str]


class AutomaticCall(TypedDict):
    session: Session
    model: str


class TransformCall(TypedDict):
    session: Session
    model: str
    message: JsonValue
    session_configuration_request: NotRequired[str | None]


class Input(TypedDict):
    calls: list[Call]


# litellm's config class carries an untyped headers dict, so the protocol is the declared shape
@runtime_checkable
class RealtimeConfig(Protocol):
    def get_complete_url(
        self, api_base: str | None, model: str, api_key: str | None = None
    ) -> str: ...

    def validate_environment(
        self, headers: dict[str, str], model: str, api_key: str | None = None
    ) -> dict[str, str]: ...

    def get_audio_mime_type(self, input_audio_format: str = "pcm16") -> str: ...

    def session_configuration_request(self, model: str) -> str: ...

    def transform_realtime_request(
        self, message: str, model: str, session_configuration_request: str | None = None
    ) -> list[str]: ...


ARBITRARY_TYPES = ConfigDict(arbitrary_types_allowed=True)
REALTIME: Final[TypeAdapter[RealtimeConfig]] = TypeAdapter[RealtimeConfig](
    RealtimeConfig, config=ARBITRARY_TYPES
)
INPUT = TypeAdapter(Input)
ENDPOINT = TypeAdapter(EndpointCall)
MIME = TypeAdapter(MimeCall)
AUTOMATIC = TypeAdapter(AutomaticCall)
TRANSFORM = TypeAdapter(TransformCall)
ENVIRONMENT = TypeAdapter(dict[str, str])
OBJECT = TypeAdapter(dict[str, JsonValue])
DOCUMENT = TypeAdapter(JsonValue)


def config(session: Session) -> RealtimeConfig:
    concrete = VertexAIRealtimeConfig(
        access_token=session["access_token"],
        project=session["project"],
        location=session["location"],
    )
    return REALTIME.validate_python(concrete)


def headers(call: EndpointCall) -> dict[str, str]:
    value = call.get("headers")
    return {} if value is None else value


def environment(call: Call) -> dict[str, str]:
    value = call.get("env")
    return {} if value is None else value


def key_used(api_key: str | None, validated: dict[str, str]) -> bool:
    if api_key is None:
        return False
    return any(api_key in value for value in validated.values())


def endpoint(call: EndpointCall) -> JsonValue:
    realtime = config(call["session"])
    url = realtime.get_complete_url(call.get("api_base"), "", call.get("api_key"))
    validated_environment = ENVIRONMENT.validate_python(
        realtime.validate_environment(headers(call), "", call.get("api_key"))
    )
    used = key_used(call.get("api_key"), validated_environment)
    return {
        "url": url,
        "environment": dict[str, JsonValue](validated_environment.items()),
        "api_key_used": used,
    }


def mime(call: MimeCall) -> JsonValue:
    realtime = config(call["session"])
    return {"mimes": [realtime.get_audio_mime_type(name) for name in call["formats"]]}


def automatic(call: AutomaticCall) -> JsonValue:
    realtime = config(call["session"])
    envelope = OBJECT.validate_json(realtime.session_configuration_request(call["model"]))
    return {"setup": envelope.get("setup")}


def transform(call: TransformCall) -> JsonValue:
    realtime = config(call["session"])
    messages = realtime.transform_realtime_request(
        json.dumps(call["message"]), call["model"], call.get("session_configuration_request")
    )
    return {"messages": [DOCUMENT.validate_json(message) for message in messages]}


def dispatch(call: Call) -> JsonValue:
    if call["op"] == "endpoint":
        return endpoint(ENDPOINT.validate_python(call))
    if call["op"] == "mime":
        return mime(MIME.validate_python(call))
    if call["op"] == "automatic":
        return automatic(AUTOMATIC.validate_python(call))
    return transform(TRANSFORM.validate_python(call))


def label(message: str) -> str:
    if DROPPED_UPDATE in message:
        return "later_update_dropped_with_guardrail"
    return "other"


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
