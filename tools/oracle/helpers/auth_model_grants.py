import asyncio
import contextlib
import io
import sys
from types import ModuleType
from typing import Final, Literal, NotRequired, TypedDict

import litellm
from litellm.proxy._types import ProxyException, UserAPIKeyAuth
from litellm.proxy.agent_endpoints.auth import agent_access_groups
from litellm.proxy.agent_endpoints.auth.agent_access_groups import AgentAccessGroupCeiling
from litellm.proxy.auth import auth_checks, fallback_model_access
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.utils import PrismaClient, ProxyLogging
from litellm.router import Router
from litellm.types.agents import AgentResponse
from pydantic import JsonValue, TypeAdapter

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
AGENT: Final[TypeAdapter[AgentResponse]] = TypeAdapter(AgentResponse)
EMPTY_IDS: Final[frozenset[str]] = frozenset()

Row = dict[str, JsonValue]
Function = Literal["agent", "fallback"]


class Step(TypedDict):
    function: Function
    model: str
    token: NotRequired[Row]
    request: NotRequired[Row]
    enabled: NotRequired[bool]
    managed: NotRequired[Row]
    managed_ceilings: NotRequired[list[list[str]]]
    unmanaged: NotRequired[list[str]]


class Input(TypedDict):
    steps: list[Step]


class StepResult(TypedDict):
    allowed: bool
    error: dict[str, JsonValue] | None
    warnings: list[str]
    effects: list[str]
    token: dict[str, JsonValue] | None


class Policy:
    def __init__(self, policy: AgentResponse | None, effects: list[str]) -> None:
        self.policy: Final = policy
        self.effects: Final = effects

    def __call__(self, _auth: UserAPIKeyAuth | None) -> AgentResponse | None:
        self.effects.append("managed_policy")
        return self.policy


class Ceilings:
    def __init__(self, rows: list[list[str]], effects: list[str]) -> None:
        self.rows: Final = rows
        self.effects: Final = effects

    async def managed(self, agent: AgentResponse) -> tuple[AgentAccessGroupCeiling, ...]:
        self.effects.append(f"managed_ceilings:{agent.agent_id}")
        return tuple(ceiling_of(models) for models in self.rows)


class Unmanaged:
    def __init__(self, models: list[str] | None, effects: list[str]) -> None:
        self.models: Final = models
        self.effects: Final = effects

    async def __call__(self, agent_id: str) -> AgentAccessGroupCeiling | None:
        self.effects.append(f"unmanaged_ceiling:{agent_id}")
        if self.models is None:
            return None
        return ceiling_of(self.models)


class Enforcement:
    def __init__(self, *, enabled: bool) -> None:
        self.enabled: Final = enabled

    def __call__(self) -> bool:
        return self.enabled


class ServiceLog(ProxyLogging):
    def __init__(self) -> None: ...


class ProxyServer(ModuleType):
    prisma_client: PrismaClient | None
    proxy_logging_obj: ServiceLog
    user_api_key_cache: UserApiKeyCache


def ceiling_of(models: list[str]) -> AgentAccessGroupCeiling:
    return AgentAccessGroupCeiling(
        access_group_ids=("group-1",),
        models=frozenset(models),
        mcp_server_ids=EMPTY_IDS,
        agent_ids=EMPTY_IDS,
    )


def policy_of(step: Step) -> AgentResponse | None:
    row = step.get("managed")
    if row is None:
        return None
    return AGENT.validate_python(row)


def token_of(step: Step) -> UserAPIKeyAuth | None:
    row = step.get("token")
    if row is None:
        return None
    return TypeAdapter(UserAPIKeyAuth).validate_python(row)


def error_out(denial: ProxyException) -> dict[str, JsonValue]:
    return {
        "class": type(denial).__name__,
        "status": int(denial.code),
        "type": denial.type,
        "param": denial.param,
        "code": denial.code,
        "message": denial.message,
    }


def identity(token: UserAPIKeyAuth | None) -> dict[str, JsonValue] | None:
    if token is None:
        return None
    return {"team_id": token.team_id, "user_id": token.user_id}


async def agent_step(step: Step) -> StepResult:
    effects: list[str] = []
    rows = step.get("managed_ceilings")
    if rows is None:
        rows = []
    managed = Ceilings(rows, effects)
    unmanaged = Unmanaged(step.get("unmanaged"), effects)
    valid_token = token_of(step)
    saved_policy = auth_checks.managed_agent_policy  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L81
    saved_ceilings = agent_access_groups.resolve_managed_agent_ceilings
    auth_checks.managed_agent_policy = Policy(policy_of(step), effects)  # type: ignore[attr-defined, assignment]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L81
    agent_access_groups.resolve_managed_agent_ceilings = managed.managed
    try:
        try:
            await auth_checks._check_agent_access_group_model_access(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L4546-L4585
                model=step["model"],
                valid_token=valid_token,
                llm_router=None,
                resolve_ceiling=unmanaged,
            )
            await auth_checks._check_agent_caller_model_access(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L4594-L4640
                model=step["model"],
                valid_token=valid_token,
                llm_router=None,
                prisma_client=None,
                user_api_key_cache=UserApiKeyCache(),
                proxy_logging_obj=ServiceLog(),
            )
        except ProxyException as denial:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L4325
            return {
                "allowed": False,
                "error": error_out(denial),
                "warnings": [],
                "effects": effects,
                "token": None,
            }
        return {
            "allowed": True,
            "error": None,
            "warnings": [],
            "effects": effects,
            "token": None,
        }
    finally:
        auth_checks.managed_agent_policy = saved_policy  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L81
        agent_access_groups.resolve_managed_agent_ceilings = saved_ceilings


async def fallback_step(step: Step) -> StepResult:
    request = step.get("request")
    if request is None:
        request = {}
    proxy_server = ProxyServer(PROXY_MODULE)
    proxy_server.prisma_client = None
    proxy_server.proxy_logging_obj = ServiceLog()
    proxy_server.user_api_key_cache = UserApiKeyCache()
    sys.modules[PROXY_MODULE] = proxy_server
    token = fallback_model_access._user_api_key_auth_from_request(request)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/fallback_model_access.py#L54-L62
    enabled = step.get("enabled")
    if enabled is None:
        enabled = False
    check = fallback_model_access.RouterFallbackAccessCheck(
        is_enforced=Enforcement(enabled=enabled)
    )
    allowed = await check(
        model=step["model"],
        request_kwargs=request,
        llm_router=Router(model_list=[]),
    )
    return {
        "allowed": allowed,
        "error": None,
        "warnings": [],
        "effects": [],
        "token": identity(token),
    }


async def run_step(step: Step) -> StepResult:
    if step["function"] == "agent":
        return await agent_step(step)
    return await fallback_step(step)


async def run_steps(steps: list[Step]) -> list[StepResult]:
    results: list[StepResult] = []
    for step in steps:
        results.append(await run_step(step))
    return results


def run(input_value: Input) -> list[StepResult]:
    saved_suppress = litellm.suppress_debug_info
    # LiteLLM prints provider hints to stdout, which carries the helper's answer
    litellm.suppress_debug_info = True
    try:
        with contextlib.redirect_stdout(io.StringIO()):
            return asyncio.run(run_steps(input_value["steps"]))
    finally:
        litellm.suppress_debug_info = saved_suppress
