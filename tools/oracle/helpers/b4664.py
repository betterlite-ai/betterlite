import asyncio
import base64
import json
from collections.abc import Awaitable, Callable, Mapping
from datetime import UTC, datetime
from functools import partial
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

from litellm.proxy._experimental.mcp_server.outbound_credentials.oauth_token_store import (
    OAuthToken,
    RefreshCoordinator,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.result import (
    Error,
    Ok,
    Result,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.sso_assertion_refresher import (
    RefreshFailure,
    RefreshingSSOAssertionStore,
    SSOAssertionRefresher,
    SSOClientConfig,
    sso_client_config,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.sso_assertion_store import (
    AssertionStoreUnavailable,
    SSOIdentityAssertion,
    assertion_from_sso_login,
)
from pydantic import JsonValue

TOKEN_HEADER: Final = b'{"alg":"none"}'
READ_FAILURE: Final = "the scripted row read failed"
WRITE_FAILURE: Final = "the scripted row write failed"
EXPIRY_SKEW_SECONDS: Final = 60.0
MILLISECONDS_PER_SECOND: Final = 1000.0

FailureMode: TypeAlias = Literal["none", "read", "write"]
RecordedCall: TypeAlias = tuple[str, list[tuple[str, str]], list[tuple[str, str]]]


class ClientConfigInput(TypedDict):
    function: Literal["client_config"]
    vars: dict[str, str]


class AssertionSpec(TypedDict):
    claims: dict[str, JsonValue]
    refresh_token: NotRequired[str]


class BodySpec(TypedDict, total=False):
    claims: dict[str, JsonValue]
    id_token: JsonValue
    refresh_token: JsonValue


class OkOutcome(TypedDict):
    kind: Literal["ok"]
    body: NotRequired[BodySpec]


class RejectedOutcome(TypedDict):
    kind: Literal["rejected"]
    detail: str


class UnavailableOutcome(TypedDict):
    kind: Literal["unavailable"]
    detail: str


OutcomeSpec: TypeAlias = OkOutcome | RejectedOutcome | UnavailableOutcome


class RefreshInput(TypedDict):
    function: Literal["refresh"]
    user_id: str
    assertion: AssertionSpec
    row: NotRequired[AssertionSpec]
    vars: dict[str, str]
    outcome: OutcomeSpec
    failure: NotRequired[FailureMode]


class StoreInput(TypedDict):
    function: Literal["store"]
    user_id: str
    now_ms: int
    rows: dict[str, AssertionSpec]
    fresh: NotRequired[dict[str, AssertionSpec]]
    vars: dict[str, str]
    outcome: OutcomeSpec
    loser: NotRequired[bool]
    failure: NotRequired[FailureMode]


Input: TypeAlias = ClientConfigInput | RefreshInput | StoreInput

TransportAnswer: TypeAlias = Result[Mapping[str, object], RefreshFailure]


def encode_segment(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode("ascii")


def mint_id_token(claims: Mapping[str, JsonValue]) -> str:
    payload = json.dumps(claims, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return f"{encode_segment(TOKEN_HEADER)}.{encode_segment(payload)}."


def build_assertion(spec: AssertionSpec) -> SSOIdentityAssertion:
    assertion = assertion_from_sso_login(mint_id_token(spec["claims"]), spec.get("refresh_token"))
    if assertion is None:
        raise ValueError("the case's assertion does not decode")
    return assertion


def render(assertion: SSOIdentityAssertion) -> JsonValue:
    expires_at = assertion.expires_at
    refresh_token = assertion.refresh_token
    return {
        "id_token": assertion.id_token.get_secret_value(),
        "refresh_token": None if refresh_token is None else refresh_token.get_secret_value(),
        "issuer": assertion.issuer,
        "expires_at_ms": None
        if expires_at is None
        else round(expires_at.timestamp() * MILLISECONDS_PER_SECOND),
    }


def body_object(spec: BodySpec | None) -> dict[str, JsonValue]:
    body: dict[str, JsonValue] = {}
    if spec is None:
        return body
    claims = spec.get("claims")
    raw_id_token = spec.get("id_token")
    if claims is not None and raw_id_token is not None:
        raise ValueError("a body names either claims or id_token, not both")
    if claims is not None:
        body["id_token"] = mint_id_token(claims)
    elif raw_id_token is not None:
        body["id_token"] = raw_id_token
    rotated = spec.get("refresh_token")
    if rotated is not None:
        body["refresh_token"] = rotated
    return body


def build_answer(outcome: OutcomeSpec) -> TransportAnswer:
    if outcome["kind"] == "ok":
        return Ok(body_object(outcome.get("body")))
    if outcome["kind"] == "rejected":
        return Error(RefreshFailure.of_rejected(outcome["detail"]))
    return Error(RefreshFailure.of_unavailable(outcome["detail"]))


class MemoryAccess:
    def __init__(self, rows: dict[str, SSOIdentityAssertion], failure: FailureMode) -> None:
        self.rows = rows
        self.writes: list[tuple[str, JsonValue]] = []
        self.failure = failure

    async def read(self, user_id: str) -> SSOIdentityAssertion | None:
        if self.failure == "read":
            raise AssertionStoreUnavailable(READ_FAILURE)
        return self.rows.get(user_id)

    async def write(self, user_id: str, assertion: SSOIdentityAssertion) -> None:
        if self.failure == "write":
            raise AssertionStoreUnavailable(WRITE_FAILURE)
        self.rows[user_id] = assertion
        self.writes.append((user_id, render(assertion)))


class InnerStore:
    def __init__(self, rows: dict[str, SSOIdentityAssertion]) -> None:
        self.rows = rows

    async def fetch(self, user_id: str) -> SSOIdentityAssertion | None:
        return self.rows.get(user_id)


class FixedClock:
    def __init__(self, now_ms: int) -> None:
        self.now_ms = now_ms

    def __call__(self) -> datetime:
        return datetime.fromtimestamp(self.now_ms / MILLISECONDS_PER_SECOND, tz=UTC)


class ScriptedTransport:
    def __init__(self, outcomes: list[TransportAnswer]) -> None:
        self.outcomes = outcomes
        self.calls: list[RecordedCall] = []

    async def post(
        self,
        url: str,
        form: Mapping[str, str],
        headers: Mapping[str, str],
    ) -> TransportAnswer:
        self.calls.append((url, sorted(form.items()), sorted(headers.items())))
        return self.outcomes.pop(0)


class LoserCoordinator:
    async def run(
        self,
        user_id: str,
        server_id: str,
        refresh: Callable[[], Awaitable[OAuthToken | None]],
        reread: Callable[[], Awaitable[OAuthToken | None]],
    ) -> OAuthToken | None:
        del user_id, server_id, refresh
        return await reread()


def no_coordinator() -> RefreshCoordinator | None:
    return None


def loser_coordinator() -> RefreshCoordinator | None:
    return LoserCoordinator()


def client_config_view(config: SSOClientConfig) -> JsonValue:
    return {
        "token_endpoint": config.token_endpoint,
        "client_id": config.client_id,
        "auth_method": config.auth_method,
        "client_secret_len": len(config.client_secret.get_secret_value()),
    }


def answered(
    result: str,
    detail: JsonValue,
    assertion: JsonValue,
    transport: ScriptedTransport,
    writes: list[tuple[str, JsonValue]],
) -> JsonValue:
    calls = transport.calls
    first = calls[0] if calls else None
    return {
        "result": result,
        "detail": detail,
        "assertion": assertion,
        "url": None if first is None else first[0],
        "form": [] if first is None else first[1],
        "headers": [] if first is None else first[2],
        "writes": [
            {"user_id": user_id, "assertion": rendered} for user_id, rendered in writes
        ],
        "posts": len(calls),
    }


async def refresh_view(input_value: RefreshInput) -> JsonValue:
    user_id = input_value["user_id"]
    row = input_value.get("row")
    access = MemoryAccess(
        {} if row is None else {user_id: build_assertion(row)},
        input_value.get("failure", "none"),
    )
    transport = ScriptedTransport([build_answer(input_value["outcome"])])
    refresher = SSOAssertionRefresher(
        transport,
        client_config=partial(sso_client_config, input_value["vars"]),
        read=access.read,
        write=access.write,
    )
    starting = build_assertion(input_value["assertion"])
    match await refresher.refresh(user_id, starting):
        case Ok(ok=renewed):
            return answered("renewed", None, render(renewed), transport, access.writes)
        case Error(error=failure):
            return answered(failure.kind, failure.detail, None, transport, access.writes)


async def store_view(input_value: StoreInput) -> JsonValue:
    user_id = input_value["user_id"]
    inner_rows = {key: build_assertion(spec) for key, spec in input_value["rows"].items()}
    fresh_spec = input_value.get("fresh")
    fresh_rows = (
        dict(inner_rows)
        if fresh_spec is None
        else {key: build_assertion(spec) for key, spec in fresh_spec.items()}
    )
    access = MemoryAccess(fresh_rows, input_value.get("failure", "none"))
    transport = ScriptedTransport([build_answer(input_value["outcome"])])
    refresher = SSOAssertionRefresher(
        transport,
        client_config=partial(sso_client_config, input_value["vars"]),
        read=access.read,
        write=access.write,
    )
    store = RefreshingSSOAssertionStore(
        InnerStore(inner_rows),
        refresher,
        fresh_read=access.read,
        coordinator_factory=loser_coordinator if input_value.get("loser", False) else no_coordinator,
        expiry_skew_seconds=EXPIRY_SKEW_SECONDS,
        clock=FixedClock(input_value["now_ms"]),
    )
    try:
        found = await store.fetch(user_id)
    except AssertionStoreUnavailable as unavailable:
        return answered("unavailable", str(unavailable), None, transport, access.writes)
    if found is None:
        return answered("none", None, None, transport, access.writes)
    return answered("assertion", None, render(found), transport, access.writes)


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "client_config":
        config = sso_client_config(input_value["vars"])
        return {"config": None if config is None else client_config_view(config)}
    if input_value["function"] == "refresh":
        return asyncio.run(refresh_view(input_value))
    return asyncio.run(store_view(input_value))
