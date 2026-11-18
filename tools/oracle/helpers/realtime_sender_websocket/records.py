from typing import Literal, TypeAlias, TypedDict

from litellm.types.guardrails import GuardrailEventHooks
from pydantic import JsonValue, TypeAdapter, ValidationError

WIRE: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
EVENT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
MAX_OPERATIONS = 32
MAX_FRAMES = 64
MAX_TEXT_BYTES = 16 * 1024
MAX_CAPTURE_BYTES = 1024 * 1024


class TextFrame(TypedDict):
    kind: Literal["text"]
    text: str


class BinaryFrame(TypedDict):
    kind: Literal["binary"]
    bytes: list[int]


Frame: TypeAlias = TextFrame | BinaryFrame


class ProviderDocument(TypedDict):
    requires_session_configuration: bool
    setup: list[JsonValue]
    content: list[JsonValue]
    transforms: list[list[Frame]]
    fail_transform: bool


class GuardrailDocument(TypedDict):
    custom: bool
    runs_for: list[GuardrailEventHooks]


class TextOperation(TypedDict):
    kind: Literal["send", "event", "cache"]
    text: str


class DisableOperation(TypedDict):
    kind: Literal["disable"]


Operation: TypeAlias = TextOperation | DisableOperation


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
    if len(input_value["operations"]) > MAX_OPERATIONS:
        raise ValueError("the scripted sender input is past its limits")
    provider = input_value["provider"]
    batches = [] if provider is None else provider["transforms"]
    frames = [frame for batch in batches for frame in batch]
    texts = [
        operation["text"]
        for operation in input_value["operations"]
        if operation["kind"] != "disable"
    ]
    texts.append(input_value["model"])
    forced = input_value["force_transcription_model"]
    if forced is not None:
        texts.append(forced)
    sizes = [len(text.encode("utf-8")) for text in texts] + [
        len(frame["text"].encode("utf-8")) if frame["kind"] == "text" else len(frame["bytes"])
        for frame in frames
    ]
    if (
        len(batches) > MAX_OPERATIONS
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


class SendFailureError(Exception): ...


class Capture:
    def __init__(self) -> None:
        self.size = 0

    def reserve(self, sizes: list[int], label: str) -> None:
        total = self.size + sum(sizes)
        if any(size > MAX_TEXT_BYTES for size in sizes) or total > MAX_CAPTURE_BYTES:
            raise SendFailureError(label)
        self.size = total
