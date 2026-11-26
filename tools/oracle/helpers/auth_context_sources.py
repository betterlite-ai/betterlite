import asyncio
import json
import os
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

import litellm
from litellm.constants import (
    CLIENT_REQUESTED_MODEL_SCOPE_KEY,
    MINIMUM_CUSTOM_KEY_LENGTH,
    MODEL_GROUP_ALIAS_RESOLVED_SCOPE_KEY,
)
from litellm.proxy import proxy_server
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.auth import auth_utils, user_api_key_auth
from litellm.proxy.common_utils.model_listing_utils import claude_code_requested_group
from litellm.router import Router
from pydantic import JsonValue, TypeAdapter
from starlette.requests import Request
from starlette.types import Message, Scope

_JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
FunctionName: TypeAlias = Literal[
    "sso", "abbreviate", "budget_names", "claude", "router_settings", "router_alias"
]
TeamLookup: TypeAlias = Literal["present", "absent", "error"]


class SsoScenario(TypedDict):
    microsoft_client_id: NotRequired[str | None]
    microsoft_client_secret: NotRequired[str | None]
    microsoft_tenant: NotRequired[str | None]
    google_client_id: NotRequired[str | None]
    google_client_secret: NotRequired[str | None]
    generic_client_id: NotRequired[str | None]
    generic_client_secret: NotRequired[str | None]
    generic_authorization_endpoint: NotRequired[str | None]
    generic_token_endpoint: NotRequired[str | None]
    generic_userinfo_endpoint: NotRequired[str | None]
    saml_idp_metadata_url: NotRequired[str | None]
    saml_idp_metadata_xml: NotRequired[str | None]
    saml_available: bool


class Call(TypedDict):
    function: FunctionName
    scenarios: NotRequired[list[SsoScenario]]
    api_key: NotRequired[str]
    minimum_custom_key_length: NotRequired[int]
    model: NotRequired[str | list[str] | None]
    requested: NotRequired[str]
    global_aliases: NotRequired[dict[str, str]]
    key_aliases: NotRequired[JsonValue]
    team_model_aliases: NotRequired[JsonValue]
    hierarchical_aliases: NotRequired[JsonValue]
    router_model_group_alias: NotRequired[list[str]]
    router_model_ids: NotRequired[list[str]]
    candidate_ids: NotRequired[dict[str, list[str]]]
    team_id: NotRequired[str | None]
    key_settings: NotRequired[JsonValue]
    team_settings: NotRequired[JsonValue]
    team_lookup: NotRequired[TeamLookup]
    route: NotRequired[str]
    route_class: NotRequired[Literal["other", "llm_api"]]
    router: NotRequired[Literal["absent", "available"]]
    marker: NotRequired[Literal["fresh", "resolved"]]
    dispatch: NotRequired[Literal["direct", "passthrough"]]
    requested_model: NotRequired[str | None]
    raw_body: NotRequired[Literal["unavailable", "available"]]
    settings: NotRequired[dict[str, JsonValue] | None]
    client_requested_model: NotRequired[str | None]


class FixtureRouter:
    def __init__(self, call: Call) -> None:
        self.model_group_alias: dict[str, str] = {
            name: name for name in call["router_model_group_alias"]
        }
        self.model_ids: list[str] = list(call["router_model_ids"])
        self.candidate_ids: dict[str, list[str]] = call["candidate_ids"]

    def has_model_id(self, name: str) -> bool:
        return name in self.model_ids

    def get_candidate_model_ids_for_route(
        self, name: str, team_id: str | None
    ) -> list[str]:
        del team_id
        return self.candidate_ids.get(name, [])


class TeamRow:
    def __init__(self, router_settings: JsonValue) -> None:
        self.router_settings: JsonValue = router_settings


class TeamSource:
    def __init__(self, call: Call) -> None:
        self.call = call

    async def __call__(self, **_kwargs: object) -> TeamRow | None:
        mode = self.call["team_lookup"]
        if mode == "error":
            raise RuntimeError("the team lookup failed")
        if mode == "absent":
            return None
        return TeamRow(self.call["team_settings"])


class SettingsProxy:
    def __init__(self, settings: dict[str, JsonValue] | None) -> None:
        self.settings = settings

    async def get_hierarchical_router_settings(
        self,
        user_api_key_dict: UserAPIKeyAuth | None,
        prisma_client: object | None,
        proxy_logging_obj: object | None = None,
    ) -> dict[str, JsonValue] | None:
        del user_api_key_dict, prisma_client, proxy_logging_obj
        await asyncio.sleep(0)
        return self.settings


class BodyReceive:
    def __init__(self, body: bytes) -> None:
        self.body = body

    async def __call__(self) -> Message:
        return {"type": "http.request", "body": self.body, "more_body": False}


def run(call: Call) -> JsonValue:
    return asyncio.run(answer(call))


async def answer(call: Call) -> JsonValue:
    function = call["function"]
    if function == "sso":
        return sso_answer(call)
    if function == "abbreviate":
        return abbreviate_answer(call)
    if function == "budget_names":
        return budget_answer(call)
    if function == "claude":
        return claude_answer(call)
    if function == "router_settings":
        return await router_settings_answer(call)
    return await router_alias_answer(call)


def set_variable(name: str, value: str | None) -> None:
    if value is None:
        os.environ.pop(name, None)
    else:
        os.environ[name] = value


def apply_scenario(scenario: SsoScenario) -> None:
    set_variable("MICROSOFT_CLIENT_ID", scenario.get("microsoft_client_id"))
    set_variable("MICROSOFT_CLIENT_SECRET", scenario.get("microsoft_client_secret"))
    set_variable("MICROSOFT_TENANT", scenario.get("microsoft_tenant"))
    set_variable("GOOGLE_CLIENT_ID", scenario.get("google_client_id"))
    set_variable("GOOGLE_CLIENT_SECRET", scenario.get("google_client_secret"))
    set_variable("GENERIC_CLIENT_ID", scenario.get("generic_client_id"))
    set_variable("GENERIC_CLIENT_SECRET", scenario.get("generic_client_secret"))
    set_variable(
        "GENERIC_AUTHORIZATION_ENDPOINT", scenario.get("generic_authorization_endpoint")
    )
    set_variable("GENERIC_TOKEN_ENDPOINT", scenario.get("generic_token_endpoint"))
    set_variable("GENERIC_USERINFO_ENDPOINT", scenario.get("generic_userinfo_endpoint"))
    set_variable("SAML_IDP_METADATA_URL", scenario.get("saml_idp_metadata_url"))
    set_variable("SAML_IDP_METADATA_XML", scenario.get("saml_idp_metadata_xml"))


def sso_answer(call: Call) -> JsonValue:
    answers: list[JsonValue] = []
    for scenario in call["scenarios"]:
        apply_scenario(scenario)
        discovery = auth_utils.has_user_setup_sso()
        ready = auth_utils.is_sso_provider_fully_configured()
        answers.append({"discovery": discovery, "ready": ready})
    return _JSON.validate_python(answers)


def abbreviate_answer(call: Call) -> JsonValue:
    if call["minimum_custom_key_length"] != MINIMUM_CUSTOM_KEY_LENGTH:
        raise ValueError(
            "minimum_custom_key_length must equal the MINIMUM_CUSTOM_KEY_LENGTH the case env sets"
        )
    return _JSON.validate_python(auth_utils.abbreviate_api_key(call["api_key"]))


def budget_answer(call: Call) -> JsonValue:
    names = user_api_key_auth._get_model_names_for_budget_checks(call["model"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L296-L303
    return _JSON.validate_python(names)


def claude_answer(call: Call) -> JsonValue:
    router = FixtureRouter(call)
    litellm.model_alias_map = call["global_aliases"]
    aliases: tuple[object, ...] = (
        call["key_aliases"],
        call["team_model_aliases"],
        call["hierarchical_aliases"],
    )
    group = claude_code_requested_group(call["requested"], router, call["team_id"], aliases)  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/model_listing_utils.py#L232-L238
    return _JSON.validate_python(group)


async def router_settings_answer(call: Call) -> JsonValue:
    user = UserAPIKeyAuth.model_construct(
        team_id=call["team_id"], router_settings=call["key_settings"]
    )
    original = proxy_server.get_team_object
    proxy_server.get_team_object = TeamSource(call)  # pyright: ignore[reportAttributeAccessIssue, reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L7707-L7720
    try:
        settings = await proxy_server.proxy_config.get_hierarchical_router_settings(
            user_api_key_dict=user, prisma_client=None, proxy_logging_obj=None
        )
    finally:
        proxy_server.get_team_object = original  # pyright: ignore[reportAttributeAccessIssue, reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L7707-L7720
    return _JSON.validate_python(settings)


def build_request(call: Call) -> tuple[Request, dict[str, object]]:
    requested = call["requested_model"]
    request_data: dict[str, object] = {} if requested is None else {"model": requested}
    scope: Scope = {
        "type": "http",
        "method": "POST",
        "path": call["route"],
        "headers": [(b"content-type", b"application/json")],
        "query_string": b"",
        "path_params": {"endpoint": "provider"} if call["dispatch"] == "passthrough" else {},
    }
    if call["raw_body"] == "available":
        scope["parsed_body"] = (tuple(request_data), request_data)
    if call["marker"] == "resolved":
        scope[MODEL_GROUP_ALIAS_RESOLVED_SCOPE_KEY] = True
    client = call["client_requested_model"]
    if client is not None:
        scope[CLIENT_REQUESTED_MODEL_SCOPE_KEY] = client
    receive = BodyReceive(json.dumps(request_data).encode())
    return Request(scope=scope, receive=receive), request_data


async def router_alias_answer(call: Call) -> JsonValue:
    request, request_data = build_request(call)
    user = UserAPIKeyAuth.model_construct(team_id=call["team_id"])
    router = Router(model_list=[]) if call["router"] == "available" else None
    original_router = proxy_server.llm_router
    original_config = proxy_server.proxy_config
    proxy_server.llm_router = router
    proxy_server.proxy_config = SettingsProxy(call["settings"])  # pyright: ignore[reportAttributeAccessIssue, reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L7673-L7679
    try:
        await user_api_key_auth._resolve_router_settings_model_group_alias(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L258-L293
            request_data, user, request, call["route"]
        )
    finally:
        proxy_server.llm_router = original_router
        proxy_server.proxy_config = original_config
    new_model = request_data.get("model")
    target = (
        new_model
        if isinstance(new_model, str) and new_model != call["requested_model"]
        else None
    )
    client = (
        request.scope.get(CLIENT_REQUESTED_MODEL_SCOPE_KEY) if target is not None else None  # pyright: ignore[reportAny]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/constants.py#L356-L357
    )
    marker = request.scope.get(MODEL_GROUP_ALIAS_RESOLVED_SCOPE_KEY) is True  # pyright: ignore[reportAny]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/constants.py#L356-L357
    answer_value: dict[str, JsonValue] = {
        "marker_set": marker,
        "client_requested_model": client if isinstance(client, str) else None,
        "target": target,
    }
    return _JSON.validate_python(answer_value)
