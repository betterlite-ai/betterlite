import asyncio
import json
import os
from typing import Final, Literal, TypeAlias, TypedDict, cast

import litellm
from litellm.constants import (
    CLIENT_REQUESTED_MODEL_SCOPE_KEY as CLIENT_KEY,
)
from litellm.constants import (
    MINIMUM_CUSTOM_KEY_LENGTH,
)
from litellm.constants import (
    MODEL_GROUP_ALIAS_RESOLVED_SCOPE_KEY as MARKER_KEY,
)
from litellm.models.team import LiteLLM_TeamTableCachedObj
from litellm.proxy import proxy_server
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.auth import auth_utils, user_api_key_auth
from litellm.proxy.auth.auth_checks import TeamNotFoundError
from litellm.proxy.common_utils.model_listing_utils import claude_code_requested_group
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.utils import PrismaClient, ProxyLogging
from litellm.repositories.prisma_protocols import DatabaseClient
from litellm.router import Router
from opentelemetry.trace import Span
from pydantic import JsonValue, TypeAdapter
from starlette.requests import Request
from starlette.types import Message
from typing_extensions import override

_JSON: Final = TypeAdapter(JsonValue)
MODEL: Final = TypeAdapter(str | None)

# every argument type the patched team lookup takes, so the fake accepts its keyword calls
LookupArgument: TypeAlias = (
    str | bool | None | Span | PrismaClient | DatabaseClient | UserApiKeyCache | ProxyLogging
)

# what build_request puts in the scope: the request line plus the parsed-body and marker entries
ScopeValue: TypeAlias = (
    str
    | bool
    | list[tuple[bytes, bytes]]
    | dict[str, str]
    | tuple[tuple[str, ...], dict[str, JsonValue]]
)


class SsoScenario(TypedDict, total=False):
    microsoft_client_id: str | None
    microsoft_client_secret: str | None
    microsoft_tenant: str | None
    google_client_id: str | None
    google_client_secret: str | None
    generic_client_id: str | None
    generic_client_secret: str | None
    generic_authorization_endpoint: str | None
    generic_token_endpoint: str | None
    generic_userinfo_endpoint: str | None
    saml_idp_metadata_url: str | None
    saml_idp_metadata_xml: str | None


class SsoCall(TypedDict):
    function: Literal["sso"]
    scenarios: list[SsoScenario]


class AbbreviateCall(TypedDict):
    function: Literal["abbreviate"]
    api_key: str
    minimum_custom_key_length: int


class BudgetCall(TypedDict):
    function: Literal["budget_names"]
    model: str | list[str] | None


class ClaudeCall(TypedDict):
    function: Literal["claude"]
    requested: str
    global_aliases: dict[str, str]
    key_aliases: JsonValue
    team_model_aliases: JsonValue
    hierarchical_aliases: JsonValue
    router_model_group_alias: list[str]
    router_model_ids: list[str]
    candidate_ids: dict[str, list[str]]
    team_id: str | None


class SettingsCall(TypedDict):
    function: Literal["router_settings"]
    key_settings: JsonValue
    team_settings: dict[str, JsonValue] | None
    team_id: str | None
    team_lookup: Literal["present", "absent", "error"]


class AliasCall(TypedDict):
    function: Literal["router_alias"]
    route: str
    router_available: bool
    marker_set: bool
    passthrough: bool
    requested_model: str | None
    raw_body_available: bool
    settings: dict[str, JsonValue] | None
    client_requested_model: str | None
    team_id: str | None


Call: TypeAlias = SsoCall | AbbreviateCall | BudgetCall | ClaudeCall | SettingsCall | AliasCall


class FixtureRouter(Router):
    def __init__(self, call: ClaudeCall) -> None:
        self.model_group_alias = {name: name for name in call["router_model_group_alias"]}
        self.model_ids = call["router_model_ids"]
        self.candidate_ids = call["candidate_ids"]

    @override
    def has_model_id(self, candidate_id: str) -> bool:
        return candidate_id in self.model_ids

    @override
    def get_candidate_model_ids_for_route(  # ast-grep-ignore: no-long-names
        self, model: str, team_id: str | None = None
    ) -> frozenset[str]:
        del team_id
        candidates = self.candidate_ids.get(model)
        return frozenset() if candidates is None else frozenset(candidates)


class TeamLookup:
    def __init__(self, call: SettingsCall) -> None:
        self.team_settings = call["team_settings"]
        self.fail = call["team_lookup"] == "error"

    async def __call__(
        self, team_id: str, *_args: LookupArgument, **_kwargs: LookupArgument
    ) -> LiteLLM_TeamTableCachedObj:
        await asyncio.sleep(0)
        settings = self.team_settings
        if settings is None or self.fail:
            raise TeamNotFoundError(team_id=team_id)
        return LiteLLM_TeamTableCachedObj(team_id=team_id, router_settings=settings)


class BodyReceive:
    def __init__(self, body: bytes) -> None:
        self.body = body

    async def __call__(self) -> Message:
        return {"type": "http.request", "body": self.body, "more_body": False}


def put(name: str, value: str | None) -> None:
    if value is None:
        os.environ.pop(name, None)
    else:
        os.environ[name] = value


def apply_scenario(scenario: SsoScenario) -> None:
    put("MICROSOFT_CLIENT_ID", scenario.get("microsoft_client_id"))
    put("MICROSOFT_CLIENT_SECRET", scenario.get("microsoft_client_secret"))
    put("MICROSOFT_TENANT", scenario.get("microsoft_tenant"))
    put("GOOGLE_CLIENT_ID", scenario.get("google_client_id"))
    put("GOOGLE_CLIENT_SECRET", scenario.get("google_client_secret"))
    put("GENERIC_CLIENT_ID", scenario.get("generic_client_id"))
    put("GENERIC_CLIENT_SECRET", scenario.get("generic_client_secret"))
    put("GENERIC_AUTHORIZATION_ENDPOINT", scenario.get("generic_authorization_endpoint"))
    put("GENERIC_TOKEN_ENDPOINT", scenario.get("generic_token_endpoint"))
    put("GENERIC_USERINFO_ENDPOINT", scenario.get("generic_userinfo_endpoint"))
    put("SAML_IDP_METADATA_URL", scenario.get("saml_idp_metadata_url"))
    put("SAML_IDP_METADATA_XML", scenario.get("saml_idp_metadata_xml"))


def scenario_answer(scenario: SsoScenario) -> JsonValue:
    apply_scenario(scenario)
    return {
        "discovery": auth_utils.has_user_setup_sso(),
        "ready": auth_utils.is_sso_provider_fully_configured(),
    }


def abbreviate_answer(call: AbbreviateCall) -> JsonValue:
    if call["minimum_custom_key_length"] != MINIMUM_CUSTOM_KEY_LENGTH:
        raise ValueError("the case env sets MINIMUM_CUSTOM_KEY_LENGTH to another minimum")
    return _JSON.validate_python(auth_utils.abbreviate_api_key(call["api_key"]))


def budget_answer(call: BudgetCall) -> JsonValue:
    names = user_api_key_auth._get_model_names_for_budget_checks(call["model"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L296-L303
    return _JSON.validate_python(names)


def claude_answer(call: ClaudeCall) -> JsonValue:
    router = FixtureRouter(call)
    litellm.model_alias_map = call["global_aliases"]
    aliases = (call["key_aliases"], call["team_model_aliases"], call["hierarchical_aliases"])
    return claude_code_requested_group(
        call["requested"], router, call["team_id"], cast("tuple[object, ...]", aliases)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/model_listing_utils.py#L232-L238
    )


def key_principal(team_id: str | None, settings: JsonValue) -> UserAPIKeyAuth:
    return UserAPIKeyAuth.model_construct(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/models/verification_token.py#L64
        team_id=team_id,
        router_settings=settings,  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/models/verification_token.py#L64
    )


async def resolved_settings(call: SettingsCall) -> JsonValue:
    principal = key_principal(call["team_id"], call["key_settings"])
    lookup = TeamLookup(call)
    saved = proxy_server.get_team_object  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L348
    proxy_server.get_team_object = lookup  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L348
    try:
        settings: dict[str, JsonValue] | None = await proxy_server.proxy_config.get_hierarchical_router_settings(  # noqa: E501  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L7673-L7679
            user_api_key_dict=principal,
            prisma_client=None,
            proxy_logging_obj=None,
        )
    finally:
        proxy_server.get_team_object = saved  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L348
    return _JSON.validate_python(settings)


def build_request(
    call: AliasCall,
) -> tuple[Request, dict[str, JsonValue], dict[str, ScopeValue]]:
    requested = call["requested_model"]
    body: dict[str, JsonValue] = {} if requested is None else {"model": requested}
    path_params: dict[str, str] = {}
    if call["passthrough"]:
        path_params["endpoint"] = "provider"
    scope: dict[str, ScopeValue] = {"headers": [], "path_params": path_params}
    if call["raw_body_available"]:
        scope["parsed_body"] = (tuple(body), body)
    if call["marker_set"]:
        scope[MARKER_KEY] = True
    client = call["client_requested_model"]
    if client is not None:
        scope[CLIENT_KEY] = client
    receive = BodyReceive(json.dumps(body).encode())
    return Request(scope=scope, receive=receive), body, scope


async def alias_rewrite(call: AliasCall) -> JsonValue:
    request, body, scope = build_request(call)
    principal = key_principal(call["team_id"], call["settings"])
    router = Router(model_list=[]) if call["router_available"] else None
    saved_router = proxy_server.llm_router
    proxy_server.llm_router = router
    try:
        await user_api_key_auth._resolve_router_settings_model_group_alias(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L258-L293
            cast("dict[str, object]", body), principal, request, call["route"]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L259
        )
    finally:
        proxy_server.llm_router = saved_router
    target = MODEL.validate_python(body.get("model"))
    client = MODEL.validate_python(scope.get(CLIENT_KEY)) if target is not None else None
    return {
        "marker_set": scope.get(MARKER_KEY) is True,
        "client_requested_model": client,
        "target": target,
    }


def run(call: Call) -> JsonValue:
    if call["function"] == "sso":
        return [scenario_answer(scenario) for scenario in call["scenarios"]]
    if call["function"] == "abbreviate":
        return abbreviate_answer(call)
    if call["function"] == "budget_names":
        return budget_answer(call)
    if call["function"] == "claude":
        return claude_answer(call)
    if call["function"] == "router_settings":
        return asyncio.run(resolved_settings(call))
    return asyncio.run(alias_rewrite(call))
