from typing import Literal, TypeAlias, TypedDict

from pydantic import JsonValue, TypeAdapter

OBJECT = TypeAdapter(dict[str, JsonValue])
CHAT_REPLY: dict[str, JsonValue] = {
    "id": "chatcmpl-b532",
    "object": "chat.completion",
    "created": 1,
    "model": "m",
    "choices": [
        {
            "index": 0,
            "finish_reason": "stop",
            "message": {"role": "assistant", "content": "hi"},
        }
    ],
    "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
}
AZURE_ENDPOINT = "https://azure-b532.invalid"
AZURE_VERSION = "2024-10-21"
AZURE_KEY = "b532-key"
PROJECTED: tuple[str, ...] = (
    "model",
    "messages",
    "tools",
    "tool_choice",
    "reasoning_effort",
)


class Install(TypedDict):
    model_cost: dict[str, dict[str, JsonValue]]
    chat_models: list[str]
    text_models: list[str]
    drop_params: bool
    openai_system_messages_first: bool


class AliasStep(TypedDict):
    model: str


class AliasCall(TypedDict):
    function: Literal["gpt5_alias_catalog"]
    install: Install
    steps: list[AliasStep]


class StreamStep(TypedDict):
    model: str | None
    stream: bool | None
    provider: str | None


class StreamCall(TypedDict):
    function: Literal["native_stream"]
    install: Install
    steps: list[StreamStep]


class AllowlistCall(TypedDict):
    function: Literal["gpt5_allowlist", "o_series_allowlist"]
    install: Install
    models: list[str]


class MapStep(TypedDict):
    model: str
    non_default: dict[str, JsonValue]
    optional: dict[str, JsonValue]
    drop_params: bool
    global_drop_params: bool


class MapCall(TypedDict):
    function: Literal["parameter_mapping"]
    install: Install
    steps: list[MapStep]


class TransformStep(TypedDict):
    model: str
    messages: list[dict[str, JsonValue]]
    tools: list[dict[str, JsonValue]] | None
    tool_choice: str | None
    reasoning_effort: (
        Literal["none", "minimal", "low", "medium", "high", "xhigh", "max", "default"]
        | None
    )


class TransformCall(TypedDict):
    function: Literal["request_transform"]
    install: Install
    steps: list[TransformStep]


Input: TypeAlias = (
    AliasCall | StreamCall | AllowlistCall | MapCall | TransformCall
)
INPUT = TypeAdapter(Input)
