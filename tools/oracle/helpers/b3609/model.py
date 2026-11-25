import datetime as dt
from typing import Final, Literal, NamedTuple, TypedDict

from litellm.types.passthrough_endpoints.pass_through_endpoints import EndpointType
from litellm.types.utils import Usage
from pydantic import JsonValue, TypeAdapter

FIXED_START: Final = dt.datetime(2024, 1, 1, tzinfo=dt.UTC)
FIXED_STAMP: Final = dt.datetime(2024, 1, 2, tzinfo=dt.UTC)
PROMPT_TOKENS: Final = 10
COMPLETION_TOKENS: Final = 5
TOTAL_TOKENS: Final = 15
RESPONSE_COST: Final = 0.25
SCRIPTED_CLASS: Final = "ScriptedReadError"

EndpointName = Literal[
    "vertex-ai", "gemini", "anthropic", "openai", "tinyfish", "generic", "decisions"
]
ENDPOINTS: Final[dict[EndpointName, EndpointType]] = {
    "vertex-ai": EndpointType.VERTEX_AI,
    "gemini": EndpointType.GEMINI,
    "anthropic": EndpointType.ANTHROPIC,
    "openai": EndpointType.OPENAI,
    "tinyfish": EndpointType.TINYFISH,
    "generic": EndpointType.GENERIC,
    "decisions": EndpointType.DECISIONS,
}


class FailureSpec(TypedDict):
    message: str
    traceback: str


class Input(TypedDict):
    endpoint_type: EndpointName
    chunks: list[list[int]]
    failure: FailureSpec | None
    disconnect_after: int | None
    status_code: int
    cost_injection: bool
    request_body: dict[str, JsonValue] | None
    url_route: str
    deferred: bool
    tinyfish_poller: bool
    run_id: str | None


class Reservation(TypedDict, total=False):
    id: str
    callback_bound: bool


class LoggingParams(TypedDict):
    metadata: dict[str, Reservation]


class UsageAndCost(NamedTuple):
    usage: Usage
    cost: float


class FailureRecord(NamedTuple):
    message: str
    traceback_tail: str


class Driven(NamedTuple):
    yielded: list[list[int]]
    queued_jobs: int
    deferred_jobs: int


class ScriptedReadError(Exception):
    def __init__(self, message: str) -> None:
        super().__init__(message)


JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
PARAMS: Final[TypeAdapter[LoggingParams]] = TypeAdapter(LoggingParams)
