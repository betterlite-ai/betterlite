from typing import Literal, Protocol, TypeAlias, TypedDict

from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.realtime import RealtimeQueryParams
from pydantic import JsonValue, TypeAdapter

SslKind: TypeAlias = Literal["none", "insecure", "shared"]
FailMode: TypeAlias = Literal["invalid_status", "error"]


class QuerySetup(TypedDict):
    api_base: str | None
    query_params: dict[str, str | None] | None


class FailureSetup(TypedDict):
    status_code: int
    message: str
    close_error: str | None


class HttpSetup(TypedDict):
    api_base: str | None
    api_key: str | None
    litellm_api_base: str | None
    litellm_api_key: str | None
    litellm_openai_key: str | None
    api_version: str | None
    headers: dict[str, str]


class Input(TypedDict):
    kind: Literal["connect", "handshake", "error", "http"]
    api_key: str | None
    model: str
    scope_headers: list[tuple[str, str]]
    ssl_verification: Literal["enabled", "disabled"]
    max_message_size_bytes: int
    openai_api_base: str | None
    openai_api_key: str | None
    queries: list[QuerySetup]
    failures: list[FailureSetup]
    http_setups: list[HttpSetup]


class CloseFrame(TypedDict):
    code: int
    reason: str | None


class StreamingRecord(TypedDict):
    force_model: str | None
    normalizer_none: bool


class HeadersValidator(Protocol):
    def validate_environment(
        self, headers: dict[str, str], model: str, api_key: str | None = None
    ) -> dict[str, str]: ...


class RealtimeRunner(Protocol):
    async def async_realtime(
        self,
        model: str,
        websocket: object,
        logging_obj: Logging,
        api_base: str | None = None,
        api_key: str | None = None,
        query_params: RealtimeQueryParams | None = None,
    ) -> None: ...


QUERY: TypeAdapter[RealtimeQueryParams] = TypeAdapter(RealtimeQueryParams)
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
HEADERS: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
FORCE_MODEL: TypeAdapter[str | None] = TypeAdapter(str | None)
