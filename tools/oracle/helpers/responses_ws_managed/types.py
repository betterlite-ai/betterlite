from typing import Final, Literal, TypedDict

from pydantic import JsonValue, TypeAdapter

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class Setup(TypedDict):
    model: str
    litellm_metadata: dict[str, JsonValue] | None
    custom_llm_provider: str | None
    api_key: str | None
    api_base: str | None
    timeout: float | None
    first_message: str | None
    kwargs: dict[str, JsonValue]


class ChunkRow(TypedDict):
    kind: Literal["none", "json", "object", "broken"]
    value: JsonValue
    event_type: str | None
    dump: str | None


class ScriptRow(TypedDict):
    fail: str | None
    chunks: list[ChunkRow]
    raise_after: int | None
    raise_message: str


class QuotaRow(TypedDict):
    kind: Literal["rate_limit", "internal"]
    message: str


class SinkRow(TypedDict):
    fails_at: int | None
    message: str


class Input(TypedDict):
    setup: Setup
    frames: list[str]
    disconnect: str
    scripts: list[ScriptRow]
    quota: QuotaRow | None
    sink: SinkRow
