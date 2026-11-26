import asyncio
import sys
from types import ModuleType
from typing import Final, Literal, NotRequired, TypedDict

from litellm import BudgetExceededError
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.models.team import LiteLLM_TeamTable
from litellm.proxy._types import LiteLLM_TeamTableCachedObj, ProxyException, UserAPIKeyAuth
from litellm.proxy.auth.auth_checks import (  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L793-L885
    TeamNotFoundError,
    _guardrail_modification_check,
    check_tools_allowlist,
)
from litellm.proxy.auth.user_api_key_auth import (  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L2625-L2744
    _inherit_org_identity,
    _safe_fetch,
    _team_obj_from_token,
    _token_can_vouch_for_team,
)
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.guardrails.tool_name_extraction import extract_request_tool_names
from litellm.proxy.utils import PrismaClient
from pydantic import BaseModel, JsonValue, TypeAdapter
from starlette.exceptions import HTTPException
from typing_extensions import override

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
Row = dict[str, JsonValue]


class GuardrailCall(TypedDict):
    function: Literal["guardrail"]
    body: Row
    team: NotRequired[Row | None]


class ToolsCall(TypedDict):
    function: Literal["tools"]
    route: str
    body: Row
    token: NotRequired[Row | None]
    team: NotRequired[Row | None]


class ExtractCall(TypedDict):
    function: Literal["extract"]
    route: str
    body: Row


class TeamFromTokenCall(TypedDict):
    function: Literal["team_from_token"]
    token: Row


class VouchCall(TypedDict):
    function: Literal["vouch"]
    token: Row
    lookup: Literal["team_not_found", "degraded"]
    outage: bool


class OrgInheritCall(TypedDict):
    function: Literal["org_inherit"]
    token: Row
    team: NotRequired[Row | None]
    org: NotRequired[Row | None]
    database: bool


class SafeFetchCall(TypedDict):
    function: Literal["safe_fetch"]
    failure: Literal["proxy", "team_not_found", "no_database"]


Input = (
    GuardrailCall
    | ToolsCall
    | ExtractCall
    | TeamFromTokenCall
    | VouchCall
    | OrgInheritCall
    | SafeFetchCall
)


# the route exception's typed fields, since HTTPException.detail carries Any
class Rejection(BaseModel):
    status_code: int
    detail: str | dict[str, str]


class ProxyServer(ModuleType):
    general_settings: dict[str, JsonValue]


def guardrail(call: GuardrailCall) -> JsonValue:
    team = None if call.get("team") is None else LiteLLM_TeamTable.model_validate(call["team"])
    try:
        _guardrail_modification_check(call["body"], team)  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L793-L844
    except HTTPException as raised:
        rejected = TypeAdapter(Rejection).validate_python(raised, from_attributes=True)
        return JSON.validate_python(
            {"outcome": "denied", "status": rejected.status_code, "detail": rejected.detail}
        )
    return {"outcome": "allowed"}


def tools(call: ToolsCall) -> JsonValue:
    token = None if call.get("token") is None else UserAPIKeyAuth.model_validate(call["token"])
    team = None if call.get("team") is None else LiteLLM_TeamTable.model_validate(call["team"])
    try:
        names = extract_request_tool_names(call["route"], call["body"])
    except TypeError:
        return {"outcome": "not_iterable"}
    try:
        asyncio.run(check_tools_allowlist(call["body"], token, team, call["route"]))
    except ProxyException as denied:
        return JSON.validate_python(
            {
                "outcome": "denied",
                "names": list(names),
                "error_type": denied.type,
                "param": denied.param,
                "code": denied.code,
            }
        )
    return JSON.validate_python({"outcome": "allowed", "names": list(names)})


def extract(call: ExtractCall) -> JsonValue:
    try:
        names = extract_request_tool_names(call["route"], call["body"])
    except TypeError:
        return {"outcome": "not_iterable"}
    return JSON.validate_python({"outcome": "extracted", "names": list(names)})


def team_view(cached: LiteLLM_TeamTableCachedObj) -> JsonValue:
    return JSON.validate_python(
        {
            "team_id": cached.team_id,
            "max_budget": cached.max_budget,
            "soft_budget": cached.soft_budget,
            "model_max_budget": cached.model_max_budget,
            "spend": cached.spend,
            "tpm_limit": cached.tpm_limit,
            "rpm_limit": cached.rpm_limit,
            "tpd_limit": cached.tpd_limit,
            "blocked": cached.blocked,
            "models": cached.models,
            "metadata": cached.metadata,
            "object_permission_id": cached.object_permission_id,
        }
    )


def team_from_token(call: TeamFromTokenCall) -> JsonValue:
    token = UserAPIKeyAuth.model_validate(call["token"])
    try:
        cached = _team_obj_from_token(token)  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L2654-L2673
    except AssertionError:
        return {"error": "missing_team_id"}
    return team_view(cached)


def install_general_settings(outage: bool) -> None:
    proxy_server = ProxyServer(PROXY_MODULE)
    proxy_server.general_settings = {"allow_requests_on_db_unavailable": outage}
    sys.modules[PROXY_MODULE] = proxy_server


def vouch(call: VouchCall) -> JsonValue:
    token = UserAPIKeyAuth.model_validate(call["token"])
    install_general_settings(call["outage"])
    lookup_error: BaseException
    match call["lookup"]:
        case "team_not_found":
            lookup_error = TeamNotFoundError("team-x")
        case "degraded":
            lookup_error = RuntimeError("the team read degraded")
    return {"value": _token_can_vouch_for_team(token, lookup_error)}  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L2676-L2703


class OrgCacheFixture(InMemoryCache):
    def __init__(self, org: Row | None) -> None:
        super().__init__()
        self.org: Row | None = org

    @override
    def get_cache(self, key: str, **kwargs: JsonValue) -> JsonValue:
        del key, kwargs
        return self.org


class RefusingTable:
    async def find_unique(self, **_kwargs: JsonValue) -> None:
        raise RuntimeError("the organization should come from the cache")


class RefusingDb(PrismaWrapper):
    def __init__(self) -> None:
        self.db = self

    @override
    def __getattr__(self, name: str) -> RefusingTable:
        del name
        return RefusingTable()


class FakePrismaClient(PrismaClient):
    def __init__(self) -> None:
        self.db = RefusingDb()


def org_view(token: UserAPIKeyAuth) -> JsonValue:
    return JSON.validate_python(
        {
            "org_id": token.org_id,
            "organization_alias": token.organization_alias,
            "organization_max_budget": token.organization_max_budget,
            "organization_tpm_limit": token.organization_tpm_limit,
            "organization_rpm_limit": token.organization_rpm_limit,
            "organization_metadata": token.organization_metadata,
        }
    )


def org_inherit(call: OrgInheritCall) -> JsonValue:
    token = UserAPIKeyAuth.model_validate(call["token"])
    team = (
        None
        if call.get("team") is None
        else LiteLLM_TeamTableCachedObj.model_validate(call["team"])
    )
    fixture = OrgCacheFixture(call.get("org"))
    cache = UserApiKeyCache(in_memory_cache=fixture, default_in_memory_ttl=60.0)
    client = FakePrismaClient() if call["database"] else None
    asyncio.run(_inherit_org_identity(token, team, client, cache, None, None))  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L2706-L2744
    return org_view(token)


async def failing(failure: str) -> None:
    match failure:
        case "proxy":
            raise ProxyException(
                message="denied", type="auth_error", param=None, code=403
            )
        case "team_not_found":
            raise TeamNotFoundError("team-x")
        case "no_database":
            raise RuntimeError("the authentication database is unreachable")


def fetch_outcome(call: SafeFetchCall) -> JsonValue:
    try:
        asyncio.run(_safe_fetch("context", failing(call["failure"])))  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L2625-L2651
    except (HTTPException, ProxyException, BudgetExceededError):
        return {"outcome": "re-raised", "class": call["failure"]}
    return {"outcome": "none"}


def run(input_value: Input) -> JsonValue:
    match input_value["function"]:
        case "guardrail":
            return guardrail(input_value)
        case "tools":
            return tools(input_value)
        case "extract":
            return extract(input_value)
        case "team_from_token":
            return team_from_token(input_value)
        case "vouch":
            return vouch(input_value)
        case "org_inherit":
            return org_inherit(input_value)
        case "safe_fetch":
            return fetch_outcome(input_value)
