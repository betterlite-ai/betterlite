from typing import Literal, NotRequired, TypedDict

from pydantic import JsonValue, TypeAdapter, ValidationError

WIRE: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
EVENT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
MAX_OPERATIONS = 32
MAX_FRAMES = 64
MAX_TEXT_BYTES = 16 * 1024
MAX_CAPTURE_BYTES = 1024 * 1024


class Frame(TypedDict):
    kind: Literal["text", "binary"]
    text: NotRequired[str]
    bytes: NotRequired[list[int]]


class ProviderDocument(TypedDict):
    requires_session_configuration: bool
    setup: list[JsonValue]
    content: list[JsonValue]
    transforms: list[list[Frame]]
    fail_transform: bool


class GuardrailDocument(TypedDict):
    custom: bool
    runs_for: list[str]


class Operation(TypedDict):
    kind: str
    text: NotRequired[str]


class Input(TypedDict):
    model: str
    force_transcription_model: str | None
    client_wants_beta: bool
    backend_uses_beta_protocol: bool | None
    provider: ProviderDocument | None
    guardrails: list[GuardrailDocument]
    backend_failures: list[int]
    client_failures: list[int]
    operations: list[Operation]


def checked(input_value: Input) -> None:
    provider = input_value["provider"]
    batches = [] if provider is None else provider["transforms"]
    frames = [frame for batch in batches for frame in batch]
    texts = [operation["text"] for operation in input_value["operations"] if "text" in operation]
    sizes = [len(text) for text in texts] + [
        len(frame["text"]) if frame["kind"] == "text" else len(frame["bytes"])
        for frame in frames
    ]
    if (
        len(texts) > MAX_OPERATIONS
        or len(frames) > MAX_FRAMES
        or len(input_value["guardrails"]) > MAX_FRAMES
        or len(input_value["backend_failures"]) > MAX_FRAMES
        or len(input_value["client_failures"]) > MAX_FRAMES
        or any(size > MAX_TEXT_BYTES for size in sizes)
        or sum(sizes) > MAX_CAPTURE_BYTES
    ):
        raise ValueError("the scripted sender input is past its limits")


def recorded(wire: str, data: bytes) -> dict[str, JsonValue]:
    try:
        payload: JsonValue = {"kind": "json", "value": WIRE.validate_json(data)}
    except ValidationError:
        payload = {"kind": "text", "text": data.decode("utf-8", errors="replace")}
    return {"wire": wire, "payload": payload}


def decoded(text: str) -> JsonValue:
    try:
        return WIRE.validate_json(text)
    except ValidationError:
        return text
