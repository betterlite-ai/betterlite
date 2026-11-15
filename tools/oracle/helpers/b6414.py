import asyncio
import datetime as dt
import re
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.meta.realtime import transformation
from litellm.types.llms.meta import MuseHandshake, MuseMode
from litellm.types.llms.openai import (
    OpenAIRealtimeOutputItemDone,
    OpenAIRealtimeResponseDelta,
)
from litellm.types.realtime import ALL_DELTA_TYPES, RealtimeResponseTransformInput
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

litellm.suppress_debug_info = True

_JSON: TypeAdapter[Json] = TypeAdapter(Json)
_DELTA: TypeAdapter[OpenAIRealtimeResponseDelta] = TypeAdapter(OpenAIRealtimeResponseDelta)
_ITEM: TypeAdapter[OpenAIRealtimeOutputItemDone] = TypeAdapter(OpenAIRealtimeOutputItemDone)
_EVENT_ID: Final = re.compile(r"event_[0-9a-f]{32}")
_ITEM_ID: Final = re.compile(r"item_[0-9a-f]{32}")


class NormalizeInput(TypedDict):
    function: Literal["normalize"]
    languages: list[str]
    tokens: list[str]
    urls: list[str | None]
    models: list[str]


class SessionInput(TypedDict):
    function: Literal["session"]
    payloads: list[str]
    expected_model: str
    access_token: str
    session_id: str


class RequestInput(TypedDict):
    function: Literal["request"]
    model: str
    api_key: str | None
    env_api_key: str | None
    headers: dict[str, str]
    messages: list[str]
    setup_messages: list[dict[str, Json]]


class PacingInput(TypedDict):
    function: Literal["pacing"]
    model: str
    access_token: str
    session: str
    lengths: list[int]
    ticks: list[float]


class EventsInput(TypedDict):
    function: Literal["events"]
    mode: MuseMode
    frames: list[dict[str, Json]]


class FrameDocument(TypedDict, total=False):
    text: str
    binary: list[int]


class TransformDocument(TypedDict):
    session_configuration_request: NotRequired[str | None]
    current_output_item_id: NotRequired[str | None]
    current_response_id: NotRequired[str | None]
    current_delta_chunks: NotRequired[list[Json] | None]
    current_item_chunks: NotRequired[list[Json] | None]
    current_conversation_id: NotRequired[str | None]
    current_delta_type: NotRequired[ALL_DELTA_TYPES | None]


class ResponseInput(TypedDict):
    function: Literal["response"]
    model: str
    access_token: str
    logging_session_id: str
    messages: list[str]
    frames: list[FrameDocument]
    transform: TransformDocument


Input: TypeAlias = (
    NormalizeInput
    | SessionInput
    | RequestInput
    | PacingInput
    | EventsInput
    | ResponseInput
)


class FakeClock:
    def __init__(self, ticks: list[float]) -> None:
        self.ticks = ticks
        self.position = 0

    def __call__(self) -> float:
        if self.position >= len(self.ticks):
            return 0.0
        reading = self.ticks[self.position]
        self.position += 1
        return reading


class FakeSleep:
    def __init__(self) -> None:
        self.delays: list[float] = []

    async def __call__(self, delay: float) -> None:
        self.delays.append(delay)


def _language(value: str) -> Json:
    try:
        normalized = transformation.normalize_language(value)
    except ValueError:
        return None
    return normalized


def _token(value: str) -> Json:
    try:
        normalized = transformation.normalize_access_token(value)
    except ValueError:
        return None
    return normalized


def _url(value: str | None) -> Json:
    try:
        built = transformation.build_muse_realtime_url(value)
    except ValueError:
        return None
    return built


def _model_url(config: transformation.MetaRealtimeConfig, value: str) -> Json:
    try:
        built = config.get_complete_url(None, value)
    except ValueError:
        return None
    return built


def _normalize(input_value: NormalizeInput) -> Json:
    config = transformation.MetaRealtimeConfig()
    languages: list[Json] = [_language(value) for value in input_value["languages"]]
    tokens: list[Json] = [_token(value) for value in input_value["tokens"]]
    urls: list[Json] = [_url(value) for value in input_value["urls"]]
    models: list[Json] = [_model_url(config, value) for value in input_value["models"]]
    document = {"languages": languages, "tokens": tokens, "urls": urls, "models": models}
    return _JSON.validate_python(document)


def _handshake_document(handshake: MuseHandshake) -> Json:
    base = {
        "authorization": {"accessToken": handshake["authorization"]["accessToken"]},
        "audioEncoding": handshake["audioEncoding"],
        "model": handshake["model"],
        "mode": handshake["mode"],
        "partialMode": handshake["partialMode"],
        "emitAudioProgress": handshake["emitAudioProgress"],
    }
    if "languageBias" not in handshake:
        return _JSON.validate_python(base)
    biased = {**base, "languageBias": list(handshake["languageBias"])}
    return _JSON.validate_python(biased)


def _session_config(payload: str, input_value: SessionInput) -> Json:
    try:
        config = transformation.parse_session_update(payload, input_value["expected_model"])
    except ValueError:
        return None
    document = {
        "model": config.model,
        "mode": config.mode,
        "sample_rate": config.sample_rate,
        "language_bias": list(config.language_bias),
        "audio_encoding": config.audio_encoding,
        "bytes_per_second": config.bytes_per_second,
        "packet_bytes": config.packet_bytes,
        "max_encoded_append_bytes": config.max_encoded_append_bytes,
        "handshake": _handshake_document(config.handshake(input_value["access_token"])),
        "openai_session": config.openai_session(input_value["session_id"]),
    }
    return _JSON.validate_python(document)


def _session(input_value: SessionInput) -> Json:
    configs: list[Json] = [
        _session_config(payload, input_value) for payload in input_value["payloads"]
    ]
    return _JSON.validate_python({"configs": configs})


def _message_document(message: str | bytes) -> Json:
    match message:
        case str():
            return {"text": message}
        case bytes():
            return {"binary": list(message)}


def _frames_document(frames: tuple[str | bytes, ...]) -> Json:
    documents: list[Json] = [_message_document(frame) for frame in frames]
    return _JSON.validate_python(documents)


def _message_frames(
    config: transformation.MetaRealtimeConfig, message: str, model: str
) -> Json:
    try:
        frames = config.transform_realtime_request(message, model)
    except ValueError:
        return None
    return _frames_document(frames)


def _header_document(
    config: transformation.MetaRealtimeConfig, input_value: RequestInput
) -> Json:
    try:
        headers = config.validate_environment(
            input_value["headers"], input_value["model"], input_value["api_key"]
        )
    except ValueError:
        return None
    return _JSON.validate_python(headers)


def _request(input_value: RequestInput) -> Json:
    config = transformation.MetaRealtimeConfig()
    headers: Json = _header_document(config, input_value)
    frames: list[Json] = [
        _message_frames(config, message, input_value["model"])
        for message in input_value["messages"]
    ]
    setup = [
        config.is_setup_message(message) for message in input_value["setup_messages"]
    ]
    usage: Json = _JSON.validate_python(
        config.unbilled_usage_on_session_close(input_value["model"])
    )
    document = {"headers": headers, "frames": frames, "setup": setup, "usage": usage}
    return _JSON.validate_python(document)


async def _pace(config: transformation.MetaRealtimeConfig, lengths: list[int]) -> None:
    for length in lengths:
        await config.pace_backend_send(bytes(length))


def _pacing(input_value: PacingInput) -> Json:
    clock = FakeClock(input_value["ticks"])
    sleep = FakeSleep()
    config = transformation.MetaRealtimeConfig(monotonic=clock, sleep=sleep)
    config.validate_environment({}, input_value["model"], input_value["access_token"])
    config.transform_realtime_request(input_value["session"], input_value["model"])
    asyncio.run(_pace(config, input_value["lengths"]))
    return _JSON.validate_python({"delays": sleep.delays})


def _event_frame(
    transformer: transformation.MuseEventTransformer, mode: MuseMode, frame: dict[str, Json]
) -> Json:
    transformer.configure(
        transformation.MuseSessionConfig(
            model=transformation.MUSE_MODEL,
            mode=mode,
            sample_rate=24000,
            language_bias=(),
        )
    )
    try:
        events = transformer.transform(frame)
    except ValueError:
        return None
    return _JSON.validate_python(list(events))


def _events(input_value: EventsInput) -> Json:
    transformer = transformation.MuseEventTransformer()
    frames: list[Json] = [
        _event_frame(transformer, input_value["mode"], frame)
        for frame in input_value["frames"]
    ]
    usage: Json = _JSON.validate_python(transformer.take_unbilled_usage())
    document: Json = _JSON.validate_python({"events": frames, "usage": usage})
    return _normalized(document)


def _delta_chunks(
    value: list[Json] | None,
) -> list[OpenAIRealtimeResponseDelta] | None:
    if value is None:
        return None
    chunks: list[OpenAIRealtimeResponseDelta] = [
        _DELTA.validate_python(item) for item in value
    ]
    return chunks


def _item_chunks(
    value: list[Json] | None,
) -> list[OpenAIRealtimeOutputItemDone] | None:
    if value is None:
        return None
    chunks: list[OpenAIRealtimeOutputItemDone] = [
        _ITEM.validate_python(item) for item in value
    ]
    return chunks


def _transform_input(document: TransformDocument) -> RealtimeResponseTransformInput:
    typed = RealtimeResponseTransformInput(
        session_configuration_request=document.get("session_configuration_request"),
        current_output_item_id=document.get("current_output_item_id"),
        current_response_id=document.get("current_response_id"),
        current_delta_chunks=_delta_chunks(document.get("current_delta_chunks")),
        current_item_chunks=_item_chunks(document.get("current_item_chunks")),
        current_conversation_id=document.get("current_conversation_id"),
        current_delta_type=document.get("current_delta_type"),
    )
    return typed


def _logging(model: str) -> Logging:
    messages: list[dict[str, Json]] = []
    logging_obj = Logging(
        model=model,
        messages=messages,
        stream=True,
        call_type="realtime",
        start_time=dt.datetime(2024, 1, 1, tzinfo=dt.UTC),
        litellm_call_id="call-1",
        function_id="function-1",
    )
    return logging_obj


def _frame_message(frame: FrameDocument) -> str | bytes:
    text = frame.get("text")
    if text is not None:
        return text
    binary = frame["binary"]
    return bytes(binary)


def _response_frame(
    config: transformation.MetaRealtimeConfig,
    frame: FrameDocument,
    model: str,
    transform: RealtimeResponseTransformInput,
    logging_obj: Logging,
) -> Json:
    try:
        response = config.transform_realtime_response(
            _frame_message(frame), model, logging_obj, transform
        )
    except ValueError:
        return None
    return _JSON.validate_python(response)


def _response(input_value: ResponseInput) -> Json:
    config = transformation.MetaRealtimeConfig()
    config.validate_environment({}, input_value["model"], input_value["access_token"])
    transform = _transform_input(input_value["transform"])
    logging_obj = _logging(input_value["model"])
    messages: list[Json] = [
        _message_frames(config, message, input_value["model"])
        for message in input_value["messages"]
    ]
    frames: list[Json] = [
        _response_frame(config, frame, input_value["model"], transform, logging_obj)
        for frame in input_value["frames"]
    ]
    created = config.transform_session_created_event(
        input_value["model"], input_value["logging_session_id"]
    )
    usage: Json = _JSON.validate_python(
        config.unbilled_usage_on_session_close(input_value["model"])
    )
    document: Json = _JSON.validate_python(
        {
            "messages": messages,
            "frames": frames,
            "created": created,
            "usage": usage,
        }
    )
    return _normalized(document)


def _normalized_field(name: str, value: Json) -> Json:
    match value:
        case str() if name == "event_id" and _EVENT_ID.fullmatch(value) is not None:
            return "event_<random>"
        case str() if name == "item_id" and _ITEM_ID.fullmatch(value) is not None:
            return "item_<random>"
        case list():
            return [_normalized(item) for item in value]
        case dict():
            return {key: _normalized_field(key, item) for key, item in value.items()}
        case None | bool() | int() | float() | str():
            return value


def _normalized(value: Json) -> Json:
    match value:
        case list():
            return [_normalized(item) for item in value]
        case dict():
            return {key: _normalized_field(key, item) for key, item in value.items()}
        case None | bool() | int() | float() | str():
            return value


def run(input_value: Input) -> Json:
    answer: Json
    if input_value["function"] == "normalize":
        answer = _normalize(input_value)
    elif input_value["function"] == "session":
        answer = _session(input_value)
    elif input_value["function"] == "request":
        answer = _request(input_value)
    elif input_value["function"] == "pacing":
        answer = _pacing(input_value)
    elif input_value["function"] == "events":
        answer = _events(input_value)
    else:
        answer = _response(input_value)
    return answer
