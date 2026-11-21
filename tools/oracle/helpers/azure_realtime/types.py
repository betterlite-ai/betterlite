from typing import Literal, Protocol, TypeAlias, TypedDict

from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.realtime import RealtimeQueryParams
from pydantic import JsonValue, TypeAdapter

SslKind: TypeAlias = Literal["none", "insecure", "shared"]
FailMode: TypeAlias = Literal["invalid_status", "error"]


class QuerySetup(TypedDict):
    api_base: str | None
    api_version: str | None
    realtime_protocol: str | None
    query_params: RealtimeQueryParams | None
    api_key: str | None
    azure_ad_token: str | None


class ProtocolSetup(TypedDict):
    configured_protocol: str | None
    query_params: RealtimeQueryParams | None


class FailureSetup(TypedDict):
    status_code: int
    message: str
    send_error: str | None
    close_error: str | None
    api_key: str | None
    azure_ad_token: str | None


class HttpSetup(TypedDict):
    api_base: str | None
    api_key: str | None
    api_version: str | None
    litellm_api_base: str | None
    litellm_api_key: str | None
    ephemeral_key: str
    headers: dict[str, str]


class Input(TypedDict):
    kind: Literal["connect", "protocol", "handshake", "error", "http"]
    model: str
    scope_headers: list[tuple[str, str]]
    ssl_verification: Literal["enabled", "disabled"]
    max_message_size_bytes: int
    azure_api_base: str | None
    azure_api_key: str | None
    azure_api_version: str | None
    queries: list[QuerySetup]
    protocols: list[ProtocolSetup]
    failures: list[FailureSetup]
    http_setups: list[HttpSetup]


class Scope(TypedDict):
    headers: list[tuple[bytes, bytes]]


class CloseFrame(TypedDict):
    code: int
    reason: str | None


class ClientSocket(Protocol):
    async def send_text(self, data: str) -> None: ...

    async def receive_text(self) -> str: ...

    async def close(self, code: int = 1000, reason: str | None = None) -> None: ...


# the handler is unannotated for its return, so the runner is named through this protocol
class RealtimeRunner(Protocol):
    async def async_realtime(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/realtime/handler.py#L133-L148
        self,
        model: str,
        websocket: ClientSocket,
        logging_obj: Logging,
        api_base: str | None = None,
        api_key: str | None = None,
        api_version: str | None = None,
        azure_ad_token: str | None = None,
        client: object | None = None,
        timeout: float | None = None,
        realtime_protocol: str | None = None,
        query_params: RealtimeQueryParams | None = None,
        user_api_key_dict: object | None = None,
        litellm_metadata: dict[str, JsonValue] | None = None,
    ) -> None: ...


class RealtimeHttpConfig(Protocol):
    def get_api_base(self, api_base: str | None) -> str: ...

    def get_api_key(self, api_key: str | None) -> str: ...

    def get_complete_url(
        self, api_base: str | None, model: str, api_version: str | None = None
    ) -> str: ...

    def get_realtime_calls_url(
        self, api_base: str | None, model: str, api_version: str | None = None
    ) -> str: ...

    def get_transcription_session_url(
        self, api_base: str | None, model: str, api_version: str | None = None
    ) -> str: ...

    def validate_environment(
        self, headers: dict[str, str], model: str, api_key: str | None = None
    ) -> dict[str, str]: ...

    def get_realtime_calls_headers(self, ephemeral_key: str) -> dict[str, str]: ...


JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
HEADERS: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
