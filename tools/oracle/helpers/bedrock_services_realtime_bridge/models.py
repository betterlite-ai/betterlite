from typing import Literal, TypeAlias, TypedDict

from pydantic import JsonValue, TypeAdapter

JsonAdapter: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
PENDING_ADAPTER: TypeAdapter[str | None] = TypeAdapter(str | None)

Outcome: TypeAlias = Literal["ended", "cancelled", "error"]
ErrorKind: TypeAlias = Literal["input", "ack", "stream", "encode", "utf8", "transform"]


class ChunkBytes(TypedDict):
    kind: Literal["bytes"]
    text: str


class ChunkMark(TypedDict):
    kind: Literal["invalid_utf8", "missing_value", "missing_bytes", "empty", "end"]


Chunk: TypeAlias = ChunkBytes | ChunkMark


class Run(TypedDict):
    kind: Literal["client", "provider"]
    trace_id: str | None
    initial: str | None
    frames: list[str | None]
    chunks: list[Chunk]
    await_error: str | None
    block: bool
    cancel: bool
    bedrock_send_failures: list[int]
    receive_failures: list[int]
    client_send_failures: list[int]
    bedrock_close_error: bool


class Input(TypedDict):
    model: str
    name_ids: list[str]
    event_ids: list[str]
    runs: list[Run]


class CloseView(TypedDict):
    code: int
    reason: str | None


class ScopeView(TypedDict):
    pending_session_update: str | None
    session_committed: bool


class RunResult(TypedDict):
    outcome: Outcome
    error: ErrorKind | None
    sent_to_bedrock: list[JsonValue]
    sent_to_client: list[JsonValue]
    closed_input: bool
    closed_client: CloseView | None
    scope: ScopeView | None
    state: JsonValue | None
    logged: list[JsonValue]
