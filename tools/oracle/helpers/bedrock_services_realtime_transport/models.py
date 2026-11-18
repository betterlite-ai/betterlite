from typing import Final, NotRequired, TypedDict

from litellm import constants
from pydantic import JsonValue, TypeAdapter

COMMITTED_KEY: Final = constants.BEDROCK_REALTIME_SESSION_COMMITTED_SCOPE_KEY
FAILURE_KEY: Final = constants.BEDROCK_REALTIME_COMMITTED_FAILURE_SCOPE_KEY
PENDING_KEY: Final = constants.BEDROCK_REALTIME_PENDING_SESSION_UPDATE_SCOPE_KEY
SUCCESS_KEY: Final = constants.REALTIME_SESSION_SUCCESS_LOGGED_KEY

BAD_REQUEST: Final = 400
UNAUTHORIZED: Final = 401
COMMITTED_PREFIX: Final = "Bedrock realtime session already committed"
TRACE_ID: Final = "trace-bedrock-transport-1"

JsonAdapter: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class ScenarioView(TypedDict):
    endpoint: str | None
    region: str | None
    access_key_id: str | None
    secret_access_key: str | None
    session_token: str | None
    model_ids: list[str]
    client_types: list[str]
    bedrock_keys: list[str]
    client_close: int | None
    runtime_closed: bool
    input_closed: bool
    resolver_static: bool
    transport_injected: bool
    committed: bool
    failure_set: bool
    success_flag: bool
    outcome: str
    error_kind: str | None
    status_code: int | None


class Auth(TypedDict):
    access_key_id: str
    secret_access_key: str
    session_token: str | None


class Run(TypedDict):
    model: str
    pending: str | None
    frames: list[str]
    chunks: list[str]
    block: str
    receive_failure: int | None
    send_failure: int | None
    open_error: bool
    client_send_failure: int | None
    credentials: str
    auth: Auth | None
    api_base: str | None
    endpoint: str | None
    region: str | None
    committed: bool
    failure: str | None


class Input(TypedDict):
    runs: list[Run]


class ClientMessage(TypedDict):
    type: str


class InputFrame(TypedDict):
    event: NotRequired[dict[str, JsonValue]]


ClientMessageAdapter: TypeAdapter[ClientMessage] = TypeAdapter(ClientMessage)
InputFrameAdapter: TypeAdapter[InputFrame] = TypeAdapter(InputFrame)
