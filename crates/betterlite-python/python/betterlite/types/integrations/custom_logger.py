from collections.abc import Mapping
from typing import Final

from pydantic import BaseModel, Field, JsonValue

from betterlite.types.litellm_params import AgenticSurface

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
