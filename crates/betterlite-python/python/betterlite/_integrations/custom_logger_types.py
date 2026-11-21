from collections.abc import AsyncIterator, Awaitable, Callable, Mapping
from datetime import datetime
from typing import Final, Protocol, TypeAlias

from pydantic import BaseModel, Field, JsonValue

from betterlite._settings.env import env_number
from betterlite.types.litellm_params import AgenticSurface
from betterlite.types.utils import ModelResponseStream

# ast-grep-ignore: no-long-names
CHAT_COMPLETION_AGENTIC_SURFACE: Final[AgenticSurface] = "chat_completions"
RESPONSES_AGENTIC_SURFACE: Final[AgenticSurface] = "responses"
# ast-grep-ignore: no-long-names
CODE_INTERPRETER_INTERCEPTION_PREFIX: Final = "_code_interpreter_interception"
HEADROOM_INTERCEPTION_PREFIX: Final = "_headroom_interception"
HEADROOM_CONVERTED_STREAM_KEY: Final = f"{HEADROOM_INTERCEPTION_PREFIX}_converted_stream"
# ast-grep-ignore: no-long-names
NON_CODE_INTERPRETER_INTERCEPTION_INTERNAL_PREFIXES: Final = frozenset(
    (
        "_websearch_interception",
        "_compression_interception",
        HEADROOM_INTERCEPTION_PREFIX,
    )
)
INTERCEPTION_INTERNAL_PREFIXES: Final = frozenset(
    (
        *NON_CODE_INTERPRETER_INTERCEPTION_INTERNAL_PREFIXES,  # ast-grep-ignore: no-long-names
        CODE_INTERPRETER_INTERCEPTION_PREFIX,  # ast-grep-ignore: no-long-names
    )
)


# ast-grep-ignore: no-one-line-functions
def is_interception_internal_key(
    key: str, prefixes: frozenset[str] = INTERCEPTION_INTERNAL_PREFIXES
) -> bool:
    return any(key.startswith(prefix) for prefix in prefixes)


CONVERTED_STREAM_KEYS: Final = frozenset(
    f"{prefix}_converted_stream" for prefix in INTERCEPTION_INTERNAL_PREFIXES
)


# ast-grep-ignore: no-one-line-functions
def converted_stream_requested(params: Mapping[str, JsonValue]) -> bool:
    return any(bool(params.get(key)) for key in CONVERTED_STREAM_KEYS)


class AgenticLoopSafetyError(ValueError):
    pass


class StandardCustomLoggerInitParams(BaseModel):
    turn_off_message_logging: bool | None = False


class AgenticLoopRequestPatch(BaseModel):
    model: str | None = None
    messages: list[dict[str, JsonValue]] | None = None
    tools: list[dict[str, JsonValue]] | None = None
    max_tokens: int | None = None
    optional_params: dict[str, JsonValue] = Field(default_factory=dict)
    kwargs: dict[str, JsonValue] = Field(default_factory=dict)


class AgenticLoopPlan(BaseModel):
    run_agentic_loop: bool = False
    request_patch: AgenticLoopRequestPatch | None = None
    response_override: JsonValue | None = None
    terminate: bool = False
    stop_reason: str | None = None
    metadata: dict[str, JsonValue] = Field(default_factory=dict)


StandardLoggingPayload: TypeAlias = dict[str, JsonValue]
StandardCallbackDynamicParams: TypeAlias = dict[str, JsonValue]
StandardAuditLogPayload: TypeAlias = dict[str, JsonValue]
CallTypes: TypeAlias = str
CallTypesLiteral: TypeAlias = str
LLMResponseTypes: TypeAlias = JsonValue
VerboseFn: TypeAlias = Callable[[str], None]
InputEventCallback: TypeAlias = Callable[[dict[str, JsonValue]], JsonValue]
EventCallback: TypeAlias = Callable[
    [dict[str, JsonValue], JsonValue, datetime, datetime], JsonValue
]
AsyncInputEventCallback: TypeAlias = Callable[[dict[str, JsonValue]], Awaitable[JsonValue]]
AsyncEventCallback: TypeAlias = Callable[
    [dict[str, JsonValue], JsonValue, datetime, datetime], Awaitable[JsonValue]
]

LITELLM_METADATA_FIELD: Final = "litellm_metadata"
OLD_LITELLM_METADATA_FIELD: Final = "metadata"
CLASSIFIER_AUDIT_FIELDS: Final = ("classifier_input", "originating_request_masked")
SENSITIVE_DATA_MAX_DEPTH: Final = env_number(
    "DEFAULT_MAX_RECURSE_DEPTH_SENSITIVE_DATA_MASKER", 10, int
)
DEFAULT_BATCH_SIZE: Final = env_number("DEFAULT_BATCH_SIZE", 512, int)
DEFAULT_FLUSH_INTERVAL_SECONDS: Final = env_number(
    "DEFAULT_FLUSH_INTERVAL_SECONDS", 5, int
)


class Span(Protocol):
    ...


class UserAPIKeyAuth(Protocol):
    ...


class DualCache(Protocol):
    ...


class LiteLLMLoggingObj(Protocol):
    ...


class MCPPostCallResponseObject(Protocol):
    ...


class PreRoutingHookResponse(Protocol):
    ...


class HTTPException(Protocol):
    ...


class BaseAnthropicMessagesConfig(Protocol):
    ...


class CustomStreamWrapper(Protocol):
    def __aiter__(self) -> AsyncIterator[ModelResponseStream]:
        ...
