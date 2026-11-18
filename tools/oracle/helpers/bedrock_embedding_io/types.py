from typing import Literal, NotRequired, TypedDict

from litellm.constants import BEDROCK_EMBEDDING_PROVIDERS_LITERAL as PROVIDERS
from pydantic import JsonValue


class Keys(TypedDict):
    access_key_id: str
    secret_access_key: str
    session_token: NotRequired[str | None]


class SignedRequest(TypedDict):
    function: Literal["signed_embedding_request"]
    credentials: Keys
    url: str
    headers: dict[str, str]
    region: str
    now: int


class TimeoutRecord(TypedDict):
    connect: float
    read: float
    write: float
    pool: float


class RedirectReply(TypedDict):
    method: str
    url: str
    status: int
    headers: dict[str, str]
    body: str


class Reply(TypedDict):
    status: int
    headers: dict[str, str]
    body: str
    failure: NotRequired[Literal["timeout", "failed"] | None]
    redirect: NotRequired[RedirectReply | None]


class HttpRequest(TypedDict):
    function: Literal["provider_json_response", "http_error"]
    asynchronous: bool
    supplied: Literal["none", "sync", "async"]
    url: str
    headers: dict[str, str]
    data: dict[str, JsonValue]
    timeout: NotRequired[float | None]
    timeout_record: NotRequired[TimeoutRecord | None]
    reply: Reply


class TransformRequest(TypedDict):
    function: Literal["embedding_usage", "async_embedding_single_arn"]
    model: str
    provider: PROVIDERS
    response_list: list[dict[str, JsonValue]]
    batch_data: NotRequired[list[dict[str, JsonValue]] | None]
    is_async_invoke: NotRequired[bool]


class CredentialRequest(TypedDict):
    function: Literal["embedding_credentials"]
    environment: dict[str, str]
    optional_params: dict[str, JsonValue]
    bearer_token: NotRequired[str | None]
    credentials: Keys


Input = SignedRequest | HttpRequest | TransformRequest | CredentialRequest
