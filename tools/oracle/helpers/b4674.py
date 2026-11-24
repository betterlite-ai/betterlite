import asyncio
import json
import os
from collections.abc import Callable
from datetime import UTC, datetime, tzinfo
from typing import ClassVar, Final, Literal, Self, TypedDict, cast

from litellm.caching.dual_cache import DualCache
from litellm.caching.redis_cache import RedisCache
from litellm.proxy import proxy_server
from litellm.proxy._experimental.mcp_server import gateway_dcr_flow
from litellm.proxy._experimental.mcp_server.gateway_dcr_flow import (
    GATEWAY_AUTH_CODE_PREFIX,
    GATEWAY_DCR_CLIENT_ID_PREFIX,
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
PREFIXES: Final[dict[str, str]] = {
    "client": GATEWAY_DCR_CLIENT_ID_PREFIX, "code": GATEWAY_AUTH_CODE_PREFIX, "none": "",
}
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


SUCCESS: Final[TypeAdapter[SuccessBody]] = TypeAdapter(SuccessBody)
ERROR: Final[TypeAdapter[ErrorBody]] = TypeAdapter(ErrorBody)


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
            current = self.counts.get(key)
            count = value if current is None else current + value
        self.counts[key] = count
        return count

    async def async_get_cache(self, key: str) -> float | None:
        if self.failure:
            raise RuntimeError("the shared cache read failed")
        if self.fixed is not None:
            return self.fixed
        return self.counts.get(key)


def base_report(status: int, headers: JsonValue) -> dict[str, JsonValue]:
    return {
        "status": status,
        "error": None,
        "error_description": None,
        "cache_headers": headers,
        "client_id_issued_at": None,
        "redirect_uris": None,
        "token_endpoint_auth_method": None,
        "grant_types": None,
        "response_types": None,
    }


def error_response_report(status: int, headers: JsonValue, body: bytes) -> dict[str, JsonValue]:
    error = ERROR.validate_json(body)
    return base_report(status, headers) | {
        "error": error["error"],
        "error_description": error["error_description"],
    }


def success_response_report(status: int, headers: JsonValue, body: bytes) -> dict[str, JsonValue]:
    success = SUCCESS.validate_json(body)
    return base_report(status, headers) | {
        "client_id_issued_at": success["client_id_issued_at"],
        "redirect_uris": JSON.validate_python(success["redirect_uris"]),
        "token_endpoint_auth_method": success["token_endpoint_auth_method"],
        "grant_types": JSON.validate_python(success["grant_types"]),
        "response_types": JSON.validate_python(success["response_types"]),
    }


def response_report(response: Response) -> dict[str, JsonValue]:
    pairs: list[list[str]] = sorted(
        [name.lower(), value]
        for name, value in response.headers.items()
        if name.lower() in CACHE_HEADERS
    )
    headers = JSON.validate_python(pairs)
    body = bytes(response.body)
    if response.status_code != CREATED:
        return error_response_report(response.status_code, headers, body)
    return success_response_report(response.status_code, headers, body)


def register_report(call: RegisterCall) -> dict[str, JsonValue]:
    configured = call["native_redirect_uris"]
    if configured is None:
        os.environ.pop(NATIVE_ENV, None)
    else:
        os.environ[NATIVE_ENV] = configured
    FrozenDatetime.instant = datetime.fromtimestamp(call["now"], UTC)
    gateway_dcr_flow.datetime = FrozenDatetime  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/gateway_dcr_flow.py#L426
    scope: dict[str, str] = {"type": "http", "path": "/register"}
    request = Request(scope)
    try:
        response = asyncio.run(
            register_aggregate_client(request, call["body"], call["token_exchange_available"])
        )
    finally:
        gateway_dcr_flow.datetime = datetime  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/gateway_dcr_flow.py#L46
    return response_report(response)


def wrapped_text(scenario: OpenScenario) -> str:
    text = scenario["raw"]
    if text is None:
        payload = scenario["payload"]
        if payload is None:
            raise ValueError("an open scenario needs a payload or raw text")
        text = PREFIXES[scenario["prefix"]] + ENC(json.dumps(payload))
    wrap = scenario["wrap"]
    if wrap == "none":
        return text
    if wrap == "tamper":
        middle = len(text) // 2
        replacement = "B" if text[middle] == "A" else "A"
        return text[:middle] + replacement + text[middle + 1:]
    if wrap == "truncate":
        return text[:-1]
    return GATEWAY_AUTH_CODE_PREFIX + text.removeprefix(GATEWAY_DCR_CLIENT_ID_PREFIX)


def open_report(scenario: OpenScenario) -> dict[str, JsonValue]:
    opened = open_gateway_dcr_client(wrapped_text(scenario))
    report: dict[str, JsonValue] = {"name": scenario["name"], "opened": opened is not None}
    if opened is None:
        return report | {"redirect_uris": None, "iat": None}
    redirect_uris: list[str] = list(opened.redirect_uris)
    return report | {"redirect_uris": JSON.validate_python(redirect_uris), "iat": opened.iat}


async def claim_results(scenario: ClaimScenario) -> list[str]:
    kind = scenario["backend"]
    dual = DualCache()
    authoritative: RedisCache | None = None
    if scenario["authority"] == "shared":
        shared = SharedCache(FIXED.get(kind), scenario["ttl"], failure=kind == "faulty")
        authoritative = cast(RedisCache, shared)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L2618
    proxy_server.redis_usage_cache = authoritative
    try:
        for key in scenario["seed_keys"]:
            await dual.async_increment_cache(key, 1, ttl=scenario["ttl"], local_only=True)  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/dual_cache.py#L565
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


def register_section(calls: list[RegisterCall] | None) -> JsonValue:
    if calls is None:
        return None
    reports: list[dict[str, JsonValue]] = [register_report(call) for call in calls]
    return JSON.validate_python(reports)


def open_section(scenarios: list[OpenScenario] | None) -> JsonValue:
    if scenarios is None:
        return None
    reports: list[dict[str, JsonValue]] = [open_report(scenario) for scenario in scenarios]
    return JSON.validate_python(reports)


def claim_section(scenarios: list[ClaimScenario] | None) -> JsonValue:
    if scenarios is None:
        return None
    reports: list[dict[str, JsonValue]] = []
    for scenario in scenarios:
        results = asyncio.run(claim_results(scenario))
        reports.append({"name": scenario["name"], "results": JSON.validate_python(results)})
    return JSON.validate_python(reports)


def run(input_value: Input) -> JsonValue:
    os.environ["LITELLM_SALT_KEY"] = SALT
    answer: dict[str, JsonValue] = {
        "register": register_section(input_value["register"]),
        "open": open_section(input_value["open"]),
        "claims": claim_section(input_value["claims"]),
    }
    return answer
