import asyncio
import json
import os
from collections.abc import Callable
from datetime import UTC, datetime, tzinfo
from typing import ClassVar, Final, Literal, Self, TypedDict, cast

from litellm.caching.caching import DualCache
from litellm.caching.redis_cache import RedisCache
from litellm.proxy import proxy_server
from litellm.proxy._experimental.mcp_server import gateway_dcr_flow
from litellm.proxy._experimental.mcp_server.gateway_dcr_flow import (
    GATEWAY_AUTH_CODE_PREFIX,
    GATEWAY_DCR_CLIENT_ID_PREFIX,
    MAX_CLIENT_ID_LENGTH,
    _SingleUseGuard,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/gateway_dcr_flow.py#L997
    open_gateway_dcr_client,
    register_aggregate_client,
)
from litellm.proxy.common_utils.encrypt_decrypt_utils import encrypt_value_helper
from pydantic import JsonValue, TypeAdapter
from starlette.requests import Request
from starlette.responses import Response
from typing_extensions import override

SALT: Final = "b4674-oracle-salt"
NATIVE_ENV: Final = "MCP_TRUSTED_NATIVE_REDIRECT_URIS"
CREATED: Final = 201
CLIENT_PREFIX: Final = GATEWAY_DCR_CLIENT_ID_PREFIX
CODE_PREFIX: Final = GATEWAY_AUTH_CODE_PREFIX
PREFIXES: Final[dict[str, str]] = {"client": CLIENT_PREFIX, "code": CODE_PREFIX, "none": ""}
FIXED: Final[dict[str, float]] = {"fixed_zero": 0.0, "fixed_one": 1.0, "fixed_two": 2.0}
CACHE_HEADERS: Final = ("cache-control", "pragma")
ENC = cast(Callable[[str], str], encrypt_value_helper)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/encrypt_decrypt_utils.py#L126
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class RegisterCall(TypedDict):
    now: int
    body: dict[str, JsonValue]
    token_exchange_available: bool
    native_redirect_uris: str | None


class OpenScenario(TypedDict):
    name: str
    payload: dict[str, JsonValue] | None
    prefix: Literal["client", "code", "none"]
    wrap: Literal["none", "tamper", "truncate", "swap_prefix"]
    raw: str | None


class ClaimStep(TypedDict):
    action: Literal["claim", "peek"]
    key: str


class ClaimScenario(TypedDict):
    name: str
    authority: Literal["local", "shared"]
    backend: Literal["counting", "fixed_zero", "fixed_one", "fixed_two", "faulty"]
    ttl: int
    seed_keys: list[str]
    steps: list[ClaimStep]


class Input(TypedDict):
    register: list[RegisterCall] | None
    open: list[OpenScenario] | None
    claims: list[ClaimScenario] | None


class SuccessBody(TypedDict):
    client_id: str
    client_id_issued_at: int
    redirect_uris: list[str]
    token_endpoint_auth_method: str
    grant_types: list[str]
    response_types: list[str]


class ErrorBody(TypedDict):
    error: str
    error_description: str


class OpenedClient(TypedDict):
    redirect_uris: list[str]
    iat: int


SUCCESS: Final[TypeAdapter[SuccessBody]] = TypeAdapter(SuccessBody)
ERROR: Final[TypeAdapter[ErrorBody]] = TypeAdapter(ErrorBody)
OPENED: Final[TypeAdapter[OpenedClient]] = TypeAdapter(OpenedClient)


class FrozenDatetime(datetime):
    instant: ClassVar[datetime] = datetime.fromtimestamp(0, UTC)

    @classmethod
    @override
    def now(cls, tz: tzinfo | None = None) -> Self:
        return cls.fromtimestamp(cls.instant.timestamp(), tz)


class SharedCache:
    def __init__(self, fixed: float | None, ttl: int, *, failure: bool) -> None:
        self.counts: dict[str, float] = {}
        self.fixed = fixed
        self.ttl = ttl
        self.failure = failure

    async def async_increment(self, key: str, value: float, ttl: int | None = None) -> float:
        if self.failure:
            raise RuntimeError("the shared cache increment failed")
        if ttl != self.ttl:
            raise RuntimeError("the guard passed a different claim ttl")
        count = self.fixed
        if count is None:
            count = (self.counts[key] if key in self.counts else 0.0) + value
        self.counts[key] = count
        return count

    async def async_get_cache(self, key: str) -> float | None:
        if self.failure:
            raise RuntimeError("the shared cache read failed")
        if self.fixed is not None:
            return self.fixed
        return self.counts.get(key)


def response_report(response: Response) -> dict[str, JsonValue]:
    headers = sorted(
        [name.lower(), value]
        for name, value in response.headers.items()
        if name.lower() in CACHE_HEADERS
    )
    if response.status_code != CREATED:
        error = ERROR.validate_json(response.body)
        return {
            "status": response.status_code,
            "error": error["error"],
            "error_description": error["error_description"],
            "cache_headers": JSON.validate_python(headers),
            "client_id_prefix_ok": None,
            "client_id_within_cap": None,
            "issued_at_matches_client_id": None,
            "client_id_issued_at": None,
            "redirect_uris": None,
            "opened_redirect_uris": None,
            "token_endpoint_auth_method": None,
            "grant_types": None,
            "response_types": None,
        }
    success = SUCCESS.validate_json(response.body)
    opened = open_gateway_dcr_client(success["client_id"])
    client = None if opened is None else OPENED.validate_json(opened.model_dump_json())
    return {
        "status": response.status_code,
        "error": None,
        "error_description": None,
        "cache_headers": JSON.validate_python(headers),
        "client_id_prefix_ok": success["client_id"].startswith(CLIENT_PREFIX),
        "client_id_within_cap": len(success["client_id"]) <= MAX_CLIENT_ID_LENGTH,
        "issued_at_matches_client_id": (
            client is not None and client["iat"] == success["client_id_issued_at"]
        ),
        "client_id_issued_at": success["client_id_issued_at"],
        "redirect_uris": JSON.validate_python(success["redirect_uris"]),
        "opened_redirect_uris": (
            None if client is None else JSON.validate_python(client["redirect_uris"])
        ),
        "token_endpoint_auth_method": success["token_endpoint_auth_method"],
        "grant_types": JSON.validate_python(success["grant_types"]),
        "response_types": JSON.validate_python(success["response_types"]),
    }


def register_report(call: RegisterCall) -> dict[str, JsonValue]:
    configured = call["native_redirect_uris"]
    if configured is None:
        os.environ.pop(NATIVE_ENV, None)
    else:
        os.environ[NATIVE_ENV] = configured
    FrozenDatetime.instant = datetime.fromtimestamp(call["now"], UTC)
    gateway_dcr_flow.datetime = FrozenDatetime  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/gateway_dcr_flow.py#L426
    try:
        request = Request(scope={"type": "http", "path": "/register"})
        response = asyncio.run(
            register_aggregate_client(request, call["body"], call["token_exchange_available"])
        )
    finally:
        gateway_dcr_flow.datetime = datetime  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/gateway_dcr_flow.py#L46
    return response_report(response)


def sealed_text(scenario: OpenScenario) -> str:
    raw = scenario["raw"]
    if raw is not None:
        return raw
    payload = scenario["payload"]
    if payload is None:
        raise ValueError("an open scenario needs a payload or raw text")
    return PREFIXES[scenario["prefix"]] + ENC(json.dumps(payload))


def wrapped_text(scenario: OpenScenario) -> str:
    text = sealed_text(scenario)
    wrap = scenario["wrap"]
    if wrap == "none":
        return text
    if wrap == "tamper":
        middle = len(text) // 2
        replacement = "B" if text[middle] == "A" else "A"
        return text[:middle] + replacement + text[middle + 1:]
    if wrap == "truncate":
        return text[:-1]
    return CODE_PREFIX + text.removeprefix(CLIENT_PREFIX)


def open_report(scenario: OpenScenario) -> dict[str, JsonValue]:
    opened = open_gateway_dcr_client(wrapped_text(scenario))
    if opened is None:
        return {"name": scenario["name"], "opened": False, "redirect_uris": None, "iat": None}
    client = OPENED.validate_json(opened.model_dump_json())
    return {
        "name": scenario["name"],
        "opened": True,
        "redirect_uris": JSON.validate_python(client["redirect_uris"]),
        "iat": client["iat"],
    }


def shared_cache(scenario: ClaimScenario) -> SharedCache:
    kind = scenario["backend"]
    if kind == "faulty":
        return SharedCache(None, scenario["ttl"], failure=True)
    if kind == "counting":
        return SharedCache(None, scenario["ttl"], failure=False)
    return SharedCache(FIXED[kind], scenario["ttl"], failure=False)


async def claim_results(scenario: ClaimScenario) -> list[str]:
    dual = DualCache()
    authoritative: RedisCache | None = None
    if scenario["authority"] == "shared":
        shared = shared_cache(scenario)
        authoritative = cast(RedisCache, shared)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L2618
    proxy_server.redis_usage_cache = authoritative
    try:
        for key in scenario["seed_keys"]:
            await dual.async_increment_cache(key, 1, ttl=scenario["ttl"], local_only=True)
        guard = _SingleUseGuard(dual)
        results: list[str] = []
        for step in scenario["steps"]:
            if step["action"] == "claim":
                results.append(await guard.claim(step["key"], scenario["ttl"]))
            else:
                results.append(await guard.peek(step["key"]))
        return results
    finally:
        proxy_server.redis_usage_cache = None


async def claim_reports(scenarios: list[ClaimScenario]) -> list[dict[str, JsonValue]]:
    reports: list[dict[str, JsonValue]] = []
    for scenario in scenarios:
        results = await claim_results(scenario)
        reports.append({"name": scenario["name"], "results": JSON.validate_python(results)})
    return reports


def run(input_value: Input) -> JsonValue:
    os.environ["LITELLM_SALT_KEY"] = SALT
    registers = input_value["register"]
    opens = input_value["open"]
    claims = input_value["claims"]
    report: dict[str, JsonValue] = {
        "register": (
            None
            if registers is None
            else JSON.validate_python([register_report(call) for call in registers])
        ),
        "open": (
            None if opens is None else JSON.validate_python([open_report(item) for item in opens])
        ),
        "claims": (
            None
            if claims is None
            else JSON.validate_python(asyncio.run(claim_reports(claims)))
        ),
    }
    return report
