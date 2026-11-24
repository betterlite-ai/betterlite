import asyncio
import base64
import json
from collections.abc import Awaitable, Callable, Mapping
from datetime import UTC, datetime
from functools import partial
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

from litellm.proxy._experimental.mcp_server.outbound_credentials import (
    oauth_token_store,
    result,
    sso_assertion_refresher,
    sso_assertion_store,
)
from litellm.proxy._experimental.mcp_server.outbound_credentials.sso_assertion_store import (
    SSOIdentityAssertion,
)
from pydantic import JsonValue
from typing_extensions import override

TOKEN_HEADER: Final = b'{"alg":"none"}'
FailureMode: TypeAlias = Literal["read", "write"]


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


class OutcomeSpec(TypedDict):
    kind: Literal["ok", "rejected", "unavailable"]
    detail: str
    body: NotRequired[BodySpec]


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


TransportAnswer: TypeAlias = result.Result[
    Mapping[str, JsonValue], sso_assertion_refresher.RefreshFailure
]


def mint_id_token(claims: Mapping[str, JsonValue]) -> str:
    header = base64.urlsafe_b64encode(TOKEN_HEADER).rstrip(b"=").decode("ascii")
    payload_text = json.dumps(claims, sort_keys=True, separators=(",", ":")).encode("utf-8")
    payload = base64.urlsafe_b64encode(payload_text).rstrip(b"=").decode("ascii")
    return f"{header}.{payload}."


def build_assertion(spec: AssertionSpec) -> SSOIdentityAssertion:
    minted = mint_id_token(spec["claims"])
    assertion = sso_assertion_store.assertion_from_sso_login(minted, spec.get("refresh_token"))
    if assertion is None:
        raise ValueError("the case's assertion does not decode")
    return assertion


def render(assertion: SSOIdentityAssertion) -> JsonValue:
    expires_at = assertion.expires_at
    refresh_token = assertion.refresh_token
    expiry_ms = None if expires_at is None else round(expires_at.timestamp() * 1000.0)
    return {
        "id_token": assertion.id_token.get_secret_value(),
        "refresh_token": None if refresh_token is None else refresh_token.get_secret_value(),
        "issuer": assertion.issuer,
        "expires_at_ms": expiry_ms,
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
        return result.Ok(body_object(outcome.get("body")))
    if outcome["kind"] == "rejected":
        return result.Error(sso_assertion_refresher.RefreshFailure.of_rejected(outcome["detail"]))
    return result.Error(sso_assertion_refresher.RefreshFailure.of_unavailable(outcome["detail"]))


class MemoryAccess:
    def __init__(self, rows: dict[str, SSOIdentityAssertion], failure: FailureMode | None) -> None:
        self.rows = rows
        self.writes: list[tuple[str, JsonValue]] = []
        self.failure = failure

    async def read(self, user_id: str) -> SSOIdentityAssertion | None:
        if self.failure == "read":
            raise sso_assertion_store.AssertionStoreUnavailable("the scripted row read failed")
        return self.rows.get(user_id)

    async def write(self, user_id: str, assertion: SSOIdentityAssertion) -> None:
        if self.failure == "write":
            raise sso_assertion_store.AssertionStoreUnavailable("the scripted row write failed")
        self.rows[user_id] = assertion
        self.writes.append((user_id, render(assertion)))


class InnerStore(sso_assertion_store.SSOAssertionStore):
    def __init__(self, rows: dict[str, SSOIdentityAssertion]) -> None:
        self.rows = rows

    @override
    async def fetch(self, user_id: str) -> SSOIdentityAssertion | None:
        return self.rows.get(user_id)


class FixedClock:
    def __init__(self, now_ms: int) -> None:
        self.now_ms = now_ms

    def __call__(self) -> datetime:
        return datetime.fromtimestamp(self.now_ms / 1000.0, tz=UTC)


class ScriptedTransport:
    def __init__(self, outcomes: list[TransportAnswer]) -> None:
        self.outcomes = outcomes
        self.calls: list[tuple[str, list[list[str]], list[list[str]]]] = []

    async def post(
        self,
        url: str,
        form: Mapping[str, str],
        headers: Mapping[str, str],
    ) -> TransportAnswer:
        self.calls.append(
            (
                url,
                [[name, value] for name, value in sorted(form.items())],
                [[name, value] for name, value in sorted(headers.items())],
            )
        )
        return self.outcomes.pop(0)


class ScriptedCoordinator:
    def __init__(self, *, loser: bool | None) -> None:
        self.loser = loser

    def __call__(self) -> oauth_token_store.RefreshCoordinator | None:
        return self if self.loser else None

    async def run(
        self,
        user_id: str,
        server_id: str,
        refresh: Callable[[], Awaitable[oauth_token_store.OAuthToken | None]],
        reread: Callable[[], Awaitable[oauth_token_store.OAuthToken | None]],
    ) -> oauth_token_store.OAuthToken | None:
        del user_id, server_id, refresh
        return await reread()


def refresher_over(
    transport: ScriptedTransport,
    variables: dict[str, str],
    access: MemoryAccess,
) -> sso_assertion_refresher.SSOAssertionRefresher:
    return sso_assertion_refresher.SSOAssertionRefresher(
        transport,
        client_config=partial(sso_assertion_refresher.sso_client_config, variables),
        read=access.read,
        write=access.write,
    )


def answered(
    result_text: str,
    detail: JsonValue,
    assertion: JsonValue,
    transport: ScriptedTransport,
    writes: list[tuple[str, JsonValue]],
) -> JsonValue:
    calls = transport.calls
    first = calls[0] if calls else None
    answer: dict[str, JsonValue] = {}
    answer["result"] = result_text
    answer["detail"] = detail
    answer["assertion"] = assertion
    answer["posts"] = len(calls)
    answer["writes"] = [{"user_id": user_id, "assertion": shown} for user_id, shown in writes]
    answer["url"] = None if first is None else first[0]
    answer["form"] = [] if first is None else [[name, value] for name, value in first[1]]
    answer["headers"] = [] if first is None else [[name, value] for name, value in first[2]]
    return answer


async def refresh_view(input_value: RefreshInput) -> JsonValue:
    user_id = input_value["user_id"]
    row = input_value.get("row")
    rows = {} if row is None else {user_id: build_assertion(row)}
    access = MemoryAccess(rows, input_value.get("failure"))
    transport = ScriptedTransport([build_answer(input_value["outcome"])])
    refresher = refresher_over(transport, input_value["vars"], access)
    starting = build_assertion(input_value["assertion"])
    match await refresher.refresh(user_id, starting):
        case result.Ok(ok=renewed):
            return answered("renewed", None, render(renewed), transport, access.writes)
        case result.Error(error=failure):
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
    access = MemoryAccess(fresh_rows, input_value.get("failure"))
    transport = ScriptedTransport([build_answer(input_value["outcome"])])
    store = sso_assertion_refresher.RefreshingSSOAssertionStore(
        InnerStore(inner_rows),
        refresher_over(transport, input_value["vars"], access),
        fresh_read=access.read,
        coordinator_factory=ScriptedCoordinator(loser=input_value.get("loser")),
        expiry_skew_seconds=60.0,
        clock=FixedClock(input_value["now_ms"]),
    )
    try:
        found = await store.fetch(user_id)
    except sso_assertion_store.AssertionStoreUnavailable as unavailable:
        return answered("unavailable", str(unavailable), None, transport, access.writes)
    if found is None:
        return answered("none", None, None, transport, access.writes)
    return answered("assertion", None, render(found), transport, access.writes)


def run(input_value: ClientConfigInput | RefreshInput | StoreInput) -> JsonValue:
    if input_value["function"] == "client_config":
        config = sso_assertion_refresher.sso_client_config(input_value["vars"])
        if config is None:
            return {"config": None}
        return {
            "config": {
                "token_endpoint": config.token_endpoint,
                "client_id": config.client_id,
                "auth_method": config.auth_method,
                "client_secret_len": len(config.client_secret.get_secret_value()),
            }
        }
    if input_value["function"] == "refresh":
        return asyncio.run(refresh_view(input_value))
    return asyncio.run(store_view(input_value))
