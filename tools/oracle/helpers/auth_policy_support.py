import asyncio
import sys
from collections.abc import Callable
from types import ModuleType
from typing import Final, Literal, NotRequired, NoReturn, TypedDict

from litellm.caching.in_memory_cache import InMemoryCache
from litellm.exceptions import BudgetExceededError
from litellm.models.team import LiteLLM_TeamTable
from litellm.proxy._types import LiteLLM_TeamTableCachedObj, ProxyException, UserAPIKeyAuth
from litellm.proxy.auth.auth_checks import (
    TeamNotFoundError,
    _guardrail_modification_check,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L793-L844
    check_tools_allowlist,  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L847-L885
)
from litellm.proxy.auth.user_api_key_auth import (
    _inherit_org_identity,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L2706-L2744
    _safe_fetch,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L2625-L2651
    _team_obj_from_token,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L2654-L2673
    _token_can_vouch_for_team,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L2676-L2703
)
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.guardrails.tool_name_extraction import extract_request_tool_names  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/tool_name_extraction.py#L63-L87
from litellm.proxy.utils import PrismaClient
from pydantic import BaseModel, JsonValue, TypeAdapter
from starlette.exceptions import HTTPException
from typing_extensions import override

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
Row = dict[str, JsonValue]


class Call(TypedDict):
    function: Literal[
        "guardrail", "tools", "extract", "team_from_token", "vouch", "org_inherit", "safe_fetch"
    ]
    body: NotRequired[Row]
    route: NotRequired[str]
    token: NotRequired[Row | None]
    team: NotRequired[Row | None]
    org: NotRequired[Row | None]
    lookup: NotRequired[Literal["team_not_found", "degraded"]]
    failure: NotRequired[Literal["proxy", "team_not_found", "no_database"]]
    database: NotRequired[bool]
    outage: NotRequired[bool]


class TeamView(TypedDict):
    team_id: str
    max_budget: float | None
    soft_budget: float | None
    model_max_budget: dict[str, JsonValue] | None
    spend: float | None
    tpm_limit: int | None
    rpm_limit: int | None
    tpd_limit: int | None
    blocked: bool
    models: list[str]
    metadata: dict[str, JsonValue] | None
    object_permission_id: str | None


class OrgView(TypedDict):
    org_id: str | None
    organization_alias: str | None
    organization_max_budget: float | None
    organization_tpm_limit: int | None
    organization_rpm_limit: int | None
    organization_metadata: dict[str, JsonValue] | None


class Rejection(BaseModel):
    status_code: int
    detail: str | dict[str, str]


class Denial(BaseModel):
    type: str
    param: str | None
    code: int | str | None


class ProxyServer(ModuleType):
    general_settings: dict[str, JsonValue]


def token_of(call: Call) -> UserAPIKeyAuth:
    row = call.get("token")
    if row is None:
        raise ValueError("the call needs a token")
    return UserAPIKeyAuth(**row)


def guardrail(call: Call) -> JsonValue:
    body = call.get("body")
    if body is None:
        raise ValueError("the call needs a body")
    team_row = call.get("team")
    team = None if team_row is None else LiteLLM_TeamTable(**team_row)
    try:
        _guardrail_modification_check(body, team)  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L793-L844
    except HTTPException as raised:
        rejected = TypeAdapter(Rejection).validate_python(raised, from_attributes=True)
        return JSON.validate_python(
            {"outcome": "denied", "status": rejected.status_code, "detail": rejected.detail}
        )
    return {"outcome": "allowed"}


def tools(call: Call) -> JsonValue:
    body = call.get("body")
    if body is None:
        raise ValueError("the call needs a body")
    route = call.get("route")
    if route is None:
        raise ValueError("the call needs a route")
    token_row = call.get("token")
    token = None if token_row is None else UserAPIKeyAuth(**token_row)
    team_row = call.get("team")
    team = None if team_row is None else LiteLLM_TeamTable(**team_row)
    try:
        names: list[str] = extract_request_tool_names(route, body)
    except TypeError:
        return {"outcome": "not_iterable"}
    try:
        asyncio.run(check_tools_allowlist(body, token, team, route))
    except ProxyException as denied:
        rejection = TypeAdapter(Denial).validate_python(denied, from_attributes=True)
        return JSON.validate_python(
            {
                "outcome": "denied",
                "names": names,
                "error_type": rejection.type,
                "param": rejection.param,
                "code": rejection.code,
            }
        )
    return JSON.validate_python({"outcome": "allowed", "names": names})


def extract(call: Call) -> JsonValue:
    body = call.get("body")
    if body is None:
        raise ValueError("the call needs a body")
    route = call.get("route")
    if route is None:
        raise ValueError("the call needs a route")
    try:
        names: list[str] = extract_request_tool_names(route, body)
    except TypeError:
        return {"outcome": "not_iterable"}
    return JSON.validate_python({"outcome": "extracted", "names": names})


def team_from_token(call: Call) -> JsonValue:
    token = token_of(call)
    try:
        cached: LiteLLM_TeamTableCachedObj = _team_obj_from_token(token)  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L2654-L2673
    except AssertionError:
        return {"error": "missing_team_id"}
    return TypeAdapter(TeamView).validate_json(cached.model_dump_json())


def vouch(call: Call) -> JsonValue:
    token = token_of(call)
    outage = call.get("outage") is True
    lookup = call.get("lookup")
    if lookup is None:
        raise ValueError("the call needs a lookup")
    proxy_server = ProxyServer(PROXY_MODULE)
    proxy_server.general_settings = {"allow_requests_on_db_unavailable": outage}
    sys.modules[PROXY_MODULE] = proxy_server
    error: BaseException
    if lookup == "team_not_found":
        error = TeamNotFoundError("team-x")
    else:
        error = RuntimeError("the team read degraded")
    return {
        "value": _token_can_vouch_for_team(token, error)  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L2676-L2703
    }


class OrgCacheFixture(InMemoryCache):
    def __init__(self, org: Row | None) -> None:
        super().__init__()
        self.org: Row | None = org

    @override
    def get_cache(self, key: str, **kwargs: JsonValue) -> JsonValue:
        del key, kwargs
        return self.org


class RefusingDb(PrismaWrapper):
    def __init__(self) -> None:
        pass

    @override
    def __getattr__(self, name: str) -> NoReturn:
        raise AttributeError(name)


class FakePrismaClient(PrismaClient):
    def __init__(self) -> None:
        self.db = RefusingDb()


def org_inherit(call: Call) -> JsonValue:
    token = token_of(call)
    row = call.get("team")
    team = None if row is None else LiteLLM_TeamTableCachedObj(**row)
    fixture = OrgCacheFixture(call.get("org"))
    cache = UserApiKeyCache(in_memory_cache=fixture, default_in_memory_ttl=60.0)
    client = FakePrismaClient() if call.get("database") is True else None
    asyncio.run(_inherit_org_identity(token, team, client, cache, None, None))  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L2706-L2744
    return TypeAdapter(OrgView).validate_json(token.model_dump_json())


async def failing(name: str) -> None:
    if name == "proxy":
        raise ProxyException(message="denied", type="auth_error", param=None, code=403)
    if name == "no_database":
        raise RuntimeError("the authentication database is unreachable")
    raise TeamNotFoundError("team-x")


def safe_fetch(call: Call) -> JsonValue:
    failure = call.get("failure")
    if failure is None:
        raise ValueError("the call needs a failure")
    try:
        asyncio.run(_safe_fetch("context", failing(failure)))  # type: ignore[misc]  # pyright: ignore[reportPrivateUsage, reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L2625-L2651
    except (HTTPException, ProxyException, BudgetExceededError):  # type: ignore[misc]  # pyright: ignore[reportAny]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L2637-L2641
        return {"outcome": "re-raised", "class": failure}
    return {"outcome": "none"}


def run(input_value: Call) -> JsonValue:
    handlers: dict[str, Callable[[Call], JsonValue]] = {
        "guardrail": guardrail,
        "tools": tools,
        "extract": extract,
        "team_from_token": team_from_token,
        "vouch": vouch,
        "org_inherit": org_inherit,
        "safe_fetch": safe_fetch,
    }
    return handlers[input_value["function"]](input_value)
