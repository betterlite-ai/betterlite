import logging
import uuid
from typing import Literal, Protocol, TypedDict
from uuid import UUID

from litellm.llms.bedrock.realtime.transformation import BedrockRealtimeConfig
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

JsonAdapter: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
MODEL = "amazon.nova-2-sonic-v1:0"
INVALID_JSON_PREFIX = "Invalid JSON message"
UNKNOWN_TYPE_PREFIX = "Unknown message type: "


class SendStep(TypedDict):
    kind: Literal["send"]
    message: str


class CloseStep(TypedDict):
    kind: Literal["close"]


class Input(TypedDict):
    ids: list[str]
    steps: list[SendStep | CloseStep]


class RealtimeConfig(Protocol):
    prompt_started: bool
    client_audio_streamed: bool
    audio_content_name: str

    def transform_realtime_request(
        self, message: str, model: str, session_configuration_request: str | None = None
    ) -> list[str]: ...

    def session_close_messages(self) -> list[str]: ...


CONFIG_TYPE: type[RealtimeConfig] = BedrockRealtimeConfig


class FixedStd:
    def __init__(self, values: list[str]) -> None:
        self.values = [UUID(value) for value in values]

    def __call__(self) -> UUID:
        return self.values.pop(0)


class WarningLog(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.messages: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


def warning_kinds(messages: list[str]) -> list[JsonValue]:
    kinds: list[JsonValue] = []
    for message in messages:
        if message.startswith(INVALID_JSON_PREFIX):
            kinds.append("invalid_json")
        elif message.startswith(UNKNOWN_TYPE_PREFIX):
            kinds.append("unknown_type:" + message.removeprefix(UNKNOWN_TYPE_PREFIX))
    return kinds


def run(input_value: Input) -> JsonValue:
    uuid.uuid4 = FixedStd(input_value["ids"])
    config = CONFIG_TYPE()
    log = WarningLog()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(log)
    logger.setLevel(logging.WARNING)
    steps: list[JsonValue] = []
    for step in input_value["steps"]:
        log.messages.clear()
        ok = True
        messages: list[str] = []
        if step["kind"] == "send":
            try:
                messages = config.transform_realtime_request(step["message"], MODEL)
            except AttributeError:
                ok = False
        else:
            messages = config.session_close_messages()
        frames: list[JsonValue] = [JsonAdapter.validate_json(message) for message in messages]
        steps.append(
            {
                "ok": ok,
                "frames": frames,
                "warnings": warning_kinds(log.messages),
                "state": {
                    "prompt_started": config.prompt_started,
                    "client_audio_streamed": config.client_audio_streamed,
                    "audio_content_name": config.audio_content_name,
                },
            }
        )
    return {"steps": steps}
