from datetime import timezone
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

from litellm.proxy._experimental.mcp_server.outbound_credentials.sso_assertion_store import (
    SSOAssertionCache,
    SSOIdentityAssertion,
)
from pydantic import JsonValue, TypeAdapter


class AssertionInput(TypedDict):
    id_token: str
    refresh_token: NotRequired[str]
    issuer: NotRequired[str]
    expires_at: NotRequired[str]


class SeedRow(TypedDict):
    user_id: str
    assertion_b64: str


class FetchStep(TypedDict):
    step: Literal["fetch"]
    user_id: str
    invalidate_during_read: NotRequired[bool]


class FetchUncachedStep(TypedDict):
    step: Literal["fetch_uncached"]
    user_id: str


class PersistStep(TypedDict):
    step: Literal["persist"]
    user_id: str
    assertion: AssertionInput


Step: TypeAlias = FetchStep | FetchUncachedStep | PersistStep


class Failure(TypedDict):
    method: Literal["find"]
    call: int


class Input(TypedDict):
    database: bool
    rows: list[SeedRow]
    failure: Failure | None
    steps: list[Step]


class LogEntry(TypedDict):
    level: str
    message: str


class StepResult(TypedDict):
    step: str
    outcome: str
    assertion: JsonValue
    cached: JsonValue
    epoch: int
    reads: int
    writes: int


class Answer(TypedDict):
    steps: list[StepResult]
    logs: list[LogEntry]


ASSERTION_ADAPTER: Final[TypeAdapter[SSOIdentityAssertion]] = TypeAdapter(SSOIdentityAssertion)


def render_assertion(assertion: SSOIdentityAssertion) -> JsonValue:
    refresh_token = assertion.refresh_token
    expires_at = assertion.expires_at
    rendered_expires_at: str | None = None
    if expires_at is not None:
        if expires_at.tzinfo is None:
            rendered_expires_at = expires_at.replace(tzinfo=timezone.utc).isoformat()
        else:
            rendered_expires_at = expires_at.isoformat()
    return {
        "id_token": assertion.id_token.get_secret_value(),
        "refresh_token": None if refresh_token is None else refresh_token.get_secret_value(),
        "issuer": assertion.issuer,
        "expires_at": rendered_expires_at,
    }


def cached_assertion(cache: SSOAssertionCache, user_id: str) -> JsonValue:
    found = cache.get(user_id)
    return None if found is None else render_assertion(found)
