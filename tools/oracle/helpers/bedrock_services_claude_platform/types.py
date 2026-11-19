from typing import Final, NotRequired, Protocol, TypedDict, Unpack

from litellm.exceptions import AuthenticationError
from litellm.types.utils import ModelResponse
from pydantic import BaseModel, ConfigDict, TypeAdapter
from pydantic import JsonValue as Json

BETA_HEADER: Final = "anthropic-beta"
WORKSPACE_HEADER: Final = "anthropic-workspace-id"
DROP_MARKER: Final = "dropping unsupported Messages API param(s) "
IGNORE_MARKER: Final = "ignoring claude_platform_unsupported_params of type "
BILLING_MARKER: Final = "x-anthropic-billing-header"
AUTHENTICATION_ERROR: Final[type[Exception]] = AuthenticationError
EMPTY_PARAMS: Final[dict[str, Json]] = {}
EMPTY_STRINGS: Final[dict[str, str]] = {}
EMPTY_MESSAGES: Final[list[Json]] = []
MANAGED_ENV: Final = (
    "ANTHROPIC_AWS_WORKSPACE_ID",
    "ANTHROPIC_WORKSPACE_ID",
    "ANTHROPIC_AWS_BASE_URL",
    "ANTHROPIC_AWS_API_BASE",
    "ANTHROPIC_AWS_API_KEY",
)


class WorkspaceFields(TypedDict):
    optional_params: NotRequired[dict[str, Json]]
    deployment: NotRequired[dict[str, str]]
    litellm_settings: NotRequired[dict[str, str]]
    environment: NotRequired[dict[str, str]]


class WorkspaceInput(TypedDict):
    model: str
    cases: list[WorkspaceFields]


class HeaderInput(WorkspaceFields):
    model: str
    messages: NotRequired[list[Json]]
    optional_params: NotRequired[dict[str, Json]]
    litellm_params: NotRequired[dict[str, Json]]
    headers: NotRequired[dict[str, str]]
    api_key: NotRequired[str | None]
    api_base: NotRequired[str | None]


class BodyInput(TypedDict):
    model: str
    messages: list[Json]
    optional_params: NotRequired[dict[str, Json]]
    litellm_params: NotRequired[dict[str, Json]]
    headers: NotRequired[dict[str, str]]


class StreamInput(TypedDict):
    id: str
    json_mode: NotRequired[bool | None]
    lines: list[str]


class Input(TypedDict):
    workspace_id_required: NotRequired[WorkspaceInput]
    chat_platform_headers: NotRequired[HeaderInput]
    native_platform_headers: NotRequired[HeaderInput]
    chat_platform_body: NotRequired[BodyInput]
    native_platform_body: NotRequired[BodyInput]
    native_anthropic_stream: NotRequired[StreamInput]


class ValidateArguments(TypedDict):
    headers: dict[str, str]
    model: str
    messages: list[Json]
    optional_params: dict[str, Json]
    litellm_params: dict[str, Json]
    api_key: NotRequired[str | None]
    api_base: NotRequired[str | None]


class ChatValidator(Protocol):
    def __call__(self, **kwargs: Unpack[ValidateArguments]) -> dict[str, str]: ...


class NativeValidator(Protocol):
    def __call__(
        self, **kwargs: Unpack[ValidateArguments]
    ) -> tuple[dict[str, str], str | None]: ...


class ChatTransformArguments(TypedDict):
    model: str
    messages: list[Json]
    optional_params: dict[str, Json]
    litellm_params: dict[str, Json]
    headers: dict[str, str]


class ChatTransformer(Protocol):
    def __call__(self, **kwargs: Unpack[ChatTransformArguments]) -> dict[str, Json]: ...


class NativeTransformer(Protocol):
    def __call__(
        self,
        model: str,
        messages: list[Json],
        request_params: dict[str, Json],
        litellm_params: dict[str, Json],
        headers: dict[str, str],
    ) -> dict[str, Json]: ...


class LineSource(Protocol):
    def __iter__(self) -> "LineSource": ...

    def __next__(self) -> str: ...


class IteratorArguments(TypedDict):
    streaming_response: LineSource
    sync_stream: bool
    json_mode: bool | None


class IteratorFactory(Protocol):
    def __call__(self, **kwargs: Unpack[IteratorArguments]) -> "SyncSource": ...


class SyncSource(Protocol):
    response_id: str

    def __iter__(self) -> "SyncSource": ...

    def __next__(self) -> ModelResponse: ...


class SystemBlock(TypedDict):
    type: str
    text: NotRequired[Json]


class ChunkDelta(TypedDict):
    content: Json


class ChunkChoice(TypedDict):
    delta: ChunkDelta
    finish_reason: Json


class ChunkView(TypedDict):
    id: str
    model: Json
    choices: list[ChunkChoice]


class Failure(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    status_code: int
    llm_provider: str
    model: str | None


SYSTEM_BLOCKS: Final[TypeAdapter[list[SystemBlock]]] = TypeAdapter(list[SystemBlock])
CHUNK_VIEW: Final[TypeAdapter[ChunkView]] = TypeAdapter(ChunkView)
