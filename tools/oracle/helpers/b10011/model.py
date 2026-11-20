from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

from litellm.proxy._experimental.mcp_server.outbound_credentials.sso_assertion_store import (
    SSOIdentityAssertion,
)
from pydantic import JsonValue, TypeAdapter


class AssertionInput(TypedDict):
    id_token: str
    refresh_token: NotRequired[str]
    issuer: NotRequired[str]
    expires_at: NotRequired[str]


class CacheSetStep(TypedDict):
    step: Literal["cache_set"]
    user_id: str
    assertion: AssertionInput
    seen_epoch: int


class CacheGetStep(TypedDict):
    step: Literal["cache_get"]
    user_id: str


class InvalidateStep(TypedDict):
    step: Literal["invalidate"]
    user_id: str


class PersistStep(TypedDict):
    step: Literal["persist"]
    user_id: str
    assertion: AssertionInput


class RetentionStep(TypedDict):
    step: Literal["retention"]


class RetainStep(TypedDict):
    step: Literal["retain"]
    user_id: str
    assertion: AssertionInput | None


Step: TypeAlias = (
    CacheSetStep | CacheGetStep | InvalidateStep | PersistStep | RetentionStep | RetainStep
)


class Failure(TypedDict):
    method: Literal["upsert", "find_first"]
    call: int


class Input(TypedDict):
    database: bool
    config_servers: list[str | None]
    db_servers: list[str | None]
    failure: Failure | None
    steps: list[Step]


class StoredPayload(TypedDict):
    id_token: str
    refresh_token: NotRequired[str]
    issuer: NotRequired[str]
    expires_at: NotRequired[str]


class StoredEntry(TypedDict):
    user_id: str
    assertion: JsonValue


class LogEntry(TypedDict):
    level: str
    message: str


class Answer(TypedDict):
    steps: list[JsonValue]
    stored: list[StoredEntry]
    reads: int
    writes: int
    logs: list[LogEntry]


ASSERTION_ADAPTER: Final[TypeAdapter[SSOIdentityAssertion]] = TypeAdapter(SSOIdentityAssertion)
PAYLOAD_ADAPTER: Final[TypeAdapter[StoredPayload]] = TypeAdapter(StoredPayload)


def render_assertion(assertion: SSOIdentityAssertion) -> JsonValue:
    refresh_token = assertion.refresh_token
    expires_at = assertion.expires_at
    return {
        "id_token": assertion.id_token.get_secret_value(),
        "refresh_token": None if refresh_token is None else refresh_token.get_secret_value(),
        "issuer": assertion.issuer,
        "expires_at": None if expires_at is None else expires_at.isoformat(),
    }


def render_payload(plaintext: str) -> JsonValue:
    payload = PAYLOAD_ADAPTER.validate_json(plaintext)
    return {
        "id_token": payload["id_token"],
        "refresh_token": payload.get("refresh_token"),
        "issuer": payload.get("issuer"),
        "expires_at": payload.get("expires_at"),
    }
