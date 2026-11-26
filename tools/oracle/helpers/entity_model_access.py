import asyncio
import sys
from types import ModuleType
from typing import Final, Literal, NotRequired, TypedDict

import litellm
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.models.organization import LiteLLM_OrganizationTable
from litellm.models.project import LiteLLM_ProjectTable
from litellm.models.team import LiteLLM_TeamTable
from litellm.models.user import LiteLLM_UserTable
from litellm.proxy._types import ModelAccessDeniedProxyException as DeniedError
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.auth.auth_checks import (
    can_key_call_model,
    can_key_call_resolved_model,
    can_org_access_model,
    can_project_access_model,
    can_team_access_model,
    can_user_call_model,
    get_authorized_resources_from_key_access_groups,
)
from litellm.proxy.common_utils.user_api_key_cache import (
    NO_TEAM_MEMBERSHIP_SENTINEL,
    UserApiKeyCache,
    team_membership_reservation_cache_key,
)
from litellm.proxy.utils import PrismaClient
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
STRINGS: Final = TypeAdapter(str)

Row = dict[str, JsonValue]
Function = Literal["key", "team", "org", "project", "resolved", "resources", "user"]
FieldName = Literal["model_names", "mcp_server_ids", "agent_ids"]
ResourceField = Literal["access_model_names", "access_mcp_server_ids", "access_agent_ids"]


class Step(TypedDict):
    function: Function
    model: str
    database: bool
    token: NotRequired[Row]
    team: NotRequired[Row]
    org: NotRequired[Row]
    project: NotRequired[Row]
    user: NotRequired[Row]
    cache: NotRequired[dict[str, JsonValue]]
    aliases: NotRequired[dict[str, str]]
    alias_targets: NotRequired[dict[str, str]]
    resource_field: NotRequired[FieldName]


class Input(TypedDict):
    steps: list[Step]


class StepResult(TypedDict):
    allowed: bool
    error: dict[str, JsonValue] | None
    resources: list[str] | None


RESOURCE_FIELDS: Final[dict[FieldName, ResourceField]] = {
    "model_names": "access_model_names",
    "mcp_server_ids": "access_mcp_server_ids",
    "agent_ids": "access_agent_ids",
}


class Fixture(InMemoryCache):
    def __init__(self, step: Step) -> None:
        super().__init__()
        self.values: dict[str, JsonValue] = {}
        seeded = step.get("cache")
        if seeded is not None:
            self.values.update(seeded)
        key = membership_sentinel(step)
        if key is not None and key not in self.values:
            self.values[key] = NO_TEAM_MEMBERSHIP_SENTINEL

    @override
    def get_cache(self, key: str, **_kwargs: JsonValue) -> JsonValue:
        return self.values.get(key)


class KeyCache(UserApiKeyCache):
    def __init__(self, fixture: Fixture) -> None:
        super().__init__(in_memory_cache=fixture, default_in_memory_ttl=60.0)


class Database(PrismaClient):
    def __init__(self) -> None: ...


class ProxyLoggingStub:
    def __init__(self) -> None:
        self.service_logging_obj = self

    async def async_service_success_hook(self, **_kwargs: JsonValue) -> None: ...


class ProxyServer(ModuleType):
    prisma_client: PrismaClient | None
    proxy_logging_obj: ProxyLoggingStub
    user_api_key_cache: UserApiKeyCache


def membership_sentinel(step: Step) -> str | None:
    auth = step.get("token")
    if auth is None:
        return None
    user_id = auth.get("user_id")
    team_id = auth.get("team_id")
    if user_id is None or team_id is None:
        return None
    return team_membership_reservation_cache_key(
        STRINGS.validate_python(user_id), STRINGS.validate_python(team_id)
    )


def token(step: Step) -> UserAPIKeyAuth:
    row = step.get("token")
    if row is None:
        raise ValueError("the step needs a token")
    return TypeAdapter(UserAPIKeyAuth).validate_python(row)


def team_row(step: Step) -> LiteLLM_TeamTable | None:
    row = step.get("team")
    if row is None:
        return None
    return TypeAdapter(LiteLLM_TeamTable).validate_python(row)


def org_row(step: Step) -> LiteLLM_OrganizationTable | None:
    row = step.get("org")
    if row is None:
        return None
    return TypeAdapter(LiteLLM_OrganizationTable).validate_python(row)


def project_row(step: Step) -> LiteLLM_ProjectTable:
    row = step.get("project")
    if row is None:
        raise ValueError("the project step needs a project row")
    return TypeAdapter(LiteLLM_ProjectTable).validate_python(row)


def user_row(step: Step) -> LiteLLM_UserTable | None:
    row = step.get("user")
    if row is None:
        return None
    return TypeAdapter(LiteLLM_UserTable).validate_python(row)


def resource_field(step: Step) -> ResourceField:
    name = step.get("resource_field")
    if name is None:
        return RESOURCE_FIELDS["model_names"]
    return RESOURCE_FIELDS[name]


def error_out(denial: DeniedError) -> dict[str, JsonValue]:
    return {
        "class": type(denial).__name__,
        "status": int(denial.code),
        "type": denial.type,
        "param": denial.param,
        "code": denial.code,
        "message": denial.message,
    }


async def call(step: Step) -> list[str] | None:
    found: list[str] | None = None
    match step["function"]:
        case "key":
            await can_key_call_model(
                model=step["model"],
                llm_model_list=None,
                valid_token=token(step),
                llm_router=None,
            )
        case "team":
            await can_team_access_model(
                model=step["model"],
                team_object=team_row(step),
                llm_router=None,
                team_model_aliases=step.get("aliases"),
                key_model_aliases=step.get("alias_targets"),
            )
        case "org":
            can_org_access_model(
                model=step["model"],
                org_object=org_row(step),
                llm_router=None,
                team_model_aliases=step.get("aliases"),
            )
        case "project":
            can_project_access_model(
                model=step["model"],
                project_object=project_row(step),
                llm_router=None,
                key_model_aliases=step.get("alias_targets"),
            )
        case "resolved":
            await can_key_call_resolved_model(
                model=step["model"],
                llm_model_list=None,
                valid_token=token(step),
                llm_router=None,
            )
        case "resources":
            found = sorted(
                await get_authorized_resources_from_key_access_groups(
                    valid_token=token(step),
                    team_object=team_row(step),
                    resource_field=resource_field(step),
                )
            )
        case "user":
            await can_user_call_model(
                model=step["model"],
                llm_router=None,
                user_object=user_row(step),
                key_model_aliases=step.get("alias_targets"),
            )
    return found


async def run_step(step: Step) -> StepResult:
    fixture = Fixture(step)
    proxy_server = ProxyServer(PROXY_MODULE)
    proxy_server.prisma_client = Database() if step["database"] else None
    proxy_server.proxy_logging_obj = ProxyLoggingStub()
    proxy_server.user_api_key_cache = KeyCache(fixture)
    sys.modules[PROXY_MODULE] = proxy_server
    try:
        resources = await call(step)
    except DeniedError as denial:
        return {"allowed": False, "error": error_out(denial), "resources": None}
    return {"allowed": True, "error": None, "resources": resources}


async def run_steps(steps: list[Step]) -> list[StepResult]:
    results: list[StepResult] = []
    for step in steps:
        result = await run_step(step)
        results.append(result)
    return results


def run(input_value: Input) -> list[StepResult]:
    saved_suppress = litellm.suppress_debug_info
    # stops get_llm_provider printing its provider hint to stdout, the helper's reply channel
    litellm.suppress_debug_info = True
    try:
        return asyncio.run(run_steps(input_value["steps"]))
    finally:
        litellm.suppress_debug_info = saved_suppress
