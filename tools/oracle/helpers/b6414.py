import asyncio
import os
import re
from typing import Final, Literal, TypeAlias, TypedDict

import litellm
from litellm.llms.meta.realtime import transformation
from litellm.types.llms.meta import MuseHandshake, MuseMode
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

litellm.suppress_debug_info = True

_JSON: TypeAdapter[Json] = TypeAdapter(Json)
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


Input: TypeAlias = NormalizeInput | SessionInput | RequestInput | PacingInput | EventsInput

_NORMALIZE: TypeAdapter[NormalizeInput] = TypeAdapter(NormalizeInput)
_SESSION: TypeAdapter[SessionInput] = TypeAdapter(SessionInput)
_REQUEST: TypeAdapter[RequestInput] = TypeAdapter(RequestInput)
_PACING: TypeAdapter[PacingInput] = TypeAdapter(PacingInput)
_EVENTS: TypeAdapter[EventsInput] = TypeAdapter(EventsInput)


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
        self.delays: list[Json] = []

    async def __call__(self, delay: float) -> None:
        self.delays.append(delay)


def language_value(value: str) -> Json:
    try:
        normalized = transformation.normalize_language(value)
    except ValueError:
        return None
    return normalized


def token_value(value: str) -> Json:
    try:
        normalized = transformation.normalize_access_token(value)
    except ValueError:
        return None
    return normalized


def url_value(value: str | None) -> Json:
    try:
        built = transformation.build_muse_realtime_url(value)
    except ValueError:
        return None
    return built


def model_value(config: transformation.MetaRealtimeConfig, value: str) -> Json:
    try:
        built = config.get_complete_url(None, value)
    except ValueError:
        return None
    return built


def normalize(input_value: NormalizeInput) -> Json:
    config = transformation.MetaRealtimeConfig()
    languages: list[Json] = [language_value(value) for value in input_value["languages"]]
    tokens: list[Json] = [token_value(value) for value in input_value["tokens"]]
    urls: list[Json] = [url_value(value) for value in input_value["urls"]]
    models: list[Json] = [model_value(config, value) for value in input_value["models"]]
    document = {"languages": languages, "tokens": tokens, "urls": urls, "models": models}
    return _JSON.validate_python(document)


def handshake_document(handshake: MuseHandshake) -> Json:
    language_bias = handshake.get("languageBias")
    if language_bias is None:
        return _JSON.validate_python(handshake)
    biased = {**handshake, "languageBias": list(language_bias)}
    return _JSON.validate_python(biased)


def session_config(payload: str, input_value: SessionInput) -> Json:
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
        "handshake": handshake_document(config.handshake(input_value["access_token"])),
        "openai_session": config.openai_session(input_value["session_id"]),
    }
    return _JSON.validate_python(document)


def session(input_value: SessionInput) -> Json:
    configs: list[Json] = [
        session_config(payload, input_value) for payload in input_value["payloads"]
    ]
    document: Json = {"configs": configs}
    return _JSON.validate_python(document)


def message_document(message: str | bytes) -> Json:
    match message:
        case str():
            return {"text": message}
        case bytes():
            return {"binary": list(message)}


def frames_document(frames: tuple[str | bytes, ...]) -> Json:
    documents: list[Json] = [message_document(frame) for frame in frames]
    return _JSON.validate_python(documents)


def request(input_value: RequestInput) -> Json:
    config = transformation.MetaRealtimeConfig()
    env_api_key = input_value["env_api_key"]
    if env_api_key is not None:
        os.environ["META_API_KEY"] = env_api_key
    try:
        headers: Json = _JSON.validate_python(
            config.validate_environment(
                input_value["headers"], input_value["model"], input_value["api_key"]
            )
        )
    except ValueError:
        headers = None
    frames: list[Json] = []
    for message in input_value["messages"]:
        try:
            frames.append(
                frames_document(
                    config.transform_realtime_request(message, input_value["model"])
                )
            )
        except ValueError:
            frames.append(None)
    setup = [config.is_setup_message(message) for message in input_value["setup_messages"]]
    usage: Json = _JSON.validate_python(
        config.unbilled_usage_on_session_close(input_value["model"])
    )
    document = {"headers": headers, "frames": frames, "setup": setup, "usage": usage}
    return _JSON.validate_python(document)


async def pace(config: transformation.MetaRealtimeConfig, lengths: list[int]) -> None:
    for length in lengths:
        await config.pace_backend_send(bytes(length))


def pacing(input_value: PacingInput) -> Json:
    clock = FakeClock(input_value["ticks"])
    sleep = FakeSleep()
    config = transformation.MetaRealtimeConfig(monotonic=clock, sleep=sleep)
    config.validate_environment({}, input_value["model"], input_value["access_token"])
    config.transform_realtime_request(input_value["session"], input_value["model"])
    asyncio.run(pace(config, input_value["lengths"]))
    document: Json = {"delays": sleep.delays}
    return _JSON.validate_python(document)


def events(input_value: EventsInput) -> Json:
    transformer = transformation.MuseEventTransformer()
    transformer.configure(
        transformation.MuseSessionConfig(
            model=transformation.MUSE_MODEL,
            mode=input_value["mode"],
            sample_rate=24000,
            language_bias=(),
        )
    )
    frames: list[Json] = []
    for frame in input_value["frames"]:
        try:
            produced = _JSON.validate_python(list(transformer.transform(frame)))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L2076-L2079
        except ValueError:
            produced = None
        frames.append(produced)
    usage: Json = _JSON.validate_python(transformer.take_unbilled_usage())
    document: Json = {"events": frames, "usage": usage}
    return normalized(document)


def normalized_field(name: str, value: Json) -> Json:
    match value:
        case str() if name == "event_id" and _EVENT_ID.fullmatch(value) is not None:
            return "event_<random>"
        case str() if name == "item_id" and _ITEM_ID.fullmatch(value) is not None:
            return "item_<random>"
        case list():
            return [normalized(item) for item in value]
        case dict():
            return {key: normalized_field(key, item) for key, item in value.items()}
        case None | bool() | int() | float() | str():
            return value


def normalized(value: Json) -> Json:
    match value:
        case list():
            return [normalized(item) for item in value]
        case dict():
            return {key: normalized_field(key, item) for key, item in value.items()}
        case None | bool() | int() | float() | str():
            return value


def run(input_value: Input) -> Json:
    function = input_value["function"]
    if function == "normalize":
        answer = normalize(_NORMALIZE.validate_python(input_value))
    elif function == "session":
        answer = session(_SESSION.validate_python(input_value))
    elif function == "request":
        answer = request(_REQUEST.validate_python(input_value))
    elif function == "pacing":
        answer = pacing(_PACING.validate_python(input_value))
    else:
        answer = events(_EVENTS.validate_python(input_value))
    return answer
