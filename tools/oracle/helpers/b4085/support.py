import asyncio
import sys
from types import ModuleType
from typing import Awaitable, Callable, Final, TypedDict

from litellm.caching.in_memory_cache import InMemoryCache
from litellm.models.team import LiteLLM_TeamTableCachedObj
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from litellm.proxy.proxy_server import proxy_logging_obj as logging_obj
from litellm.proxy.utils import PrismaClient, ProxyLogging
from litellm.router import Router
from pydantic import JsonValue
from typing_extensions import override

from b4083.inputs import AttemptData, JobRecord, JobStat, Price
from b4083.support import (
    Counter,
    Deployment,
    Funnel,
    AttemptTable,
    deployment,
    money,
    money_or_null,
    mock_reply,
)

from .inputs import ArmInput, Input, TeamInput

PROXY_MODULE: Final = "litellm.proxy.proxy_server"
JOB_ID: Final = "job-1"
REQUEST_ID: Final = "req-1"
JUDGE_MODEL: Final = "judge-model"
SHADOW_TEXT: Final = "shadow answer"
REAL_TEXT: Final = "real answer"
KEY_HASH: Final = "key-hash"
REAL_COST: Final = 0.25
PRICE: Final[Price] = {"input_cost_per_token": 0.25, "output_cost_per_token": 0.5}
TICKS: Final = 500


class EmptyPrisma(PrismaClient):
    def __init__(self) -> None:
        pass


class ProxyServer(ModuleType):
    get_current_spend: Callable[..., Awaitable[float]]
    prisma_client: PrismaClient | None
    proxy_logging_obj: ProxyLogging
    user_api_key_cache: UserApiKeyCache


class SpendScript:
    def __init__(self, key_spend: float | None, team_spend: float | None) -> None:
        self.key_spend = key_spend
        self.team_spend = team_spend
        self.reads: list[JsonValue] = []

    async def current(
        self, counter_key: str, fallback_spend: float, max_budget: float | None = None
    ) -> float:
        self.reads.append([counter_key, money(fallback_spend), money_or_null(max_budget)])
        scripted = self.key_spend if counter_key.startswith("spend:key:") else self.team_spend
        if scripted is None:
            raise RuntimeError("counter unreadable")
        return scripted


class Reader(Counter):
    def __init__(self, spend: float | None) -> None:
        events: list[str] = []
        super().__init__(events)
        self.spend = spend
        self.reads: list[JsonValue] = []

    @override
    async def read(self, key: str, fallback: float, max_budget: float) -> float:
        self.reads.append([key, money(fallback), money(max_budget)])
        if self.spend is None:
            raise RuntimeError("eval counter unreadable")
        return self.spend


class ArmProvider:
    def __init__(self, router: Router, pipeline_error: bool) -> None:
        self.router = router
        self.pipeline_error = pipeline_error
        self.arm_phase = False
        self.raised = False

    def __call__(self) -> Router:
        if self.arm_phase and self.pipeline_error and not self.raised:
            self.raised = True
            raise RuntimeError("provider exploded")
        return self.router


class ClientProvider:
    def __init__(self, client: PrismaClient) -> None:
        self.client = client

    def __call__(self) -> PrismaClient:
        return self.client


class LoadOnlyProvider:
    def __init__(self, client: PrismaClient) -> None:
        self.client = client
        self.calls = 0

    def __call__(self) -> PrismaClient | None:
        self.calls += 1
        if self.calls == 1:
            return self.client
        return None


class Params(TypedDict):
    metadata: dict[str, JsonValue | UserAPIKeyAuth]


class RequestKwargs(TypedDict):
    standard_logging_object: dict[str, JsonValue]
    litellm_params: Params
    messages: list[JsonValue]


def judge_content(reply: str) -> str:
    if reply == "a":
        return '{"preference": "A", "confidence": 0.9}'
    if reply == "b":
        return '{"preference": "B", "confidence": 0.9}'
    if reply == "tie":
        return '{"preference": "tie", "confidence": 0.9}'
    return "not json"


def shadow_content(arm: ArmInput) -> str:
    return SHADOW_TEXT if arm["shadow"] == "reply" else ""


def arm_router(call: Input) -> Router:
    judge = deployment(JUDGE_MODEL, mock_reply(judge_content(call["judge_reply"])), PRICE)
    deployments: list[Deployment] = [judge]
    for arm in call["arms"]:
        reply = mock_reply(shadow_content(arm))
        deployments.append(deployment(arm["router"], reply, PRICE))
    return Router(model_list=deployments)


def job_record(call: Input) -> JobRecord:
    routers = [arm["router"] for arm in call["arms"]]
    return JobRecord.model_validate(
        {
            "id": JOB_ID,
            "target_type": "key",
            "target_id": KEY_HASH,
            "stopped_at": None,
            "router_name": routers[0],
            "router_names": routers if len(routers) > 1 else [],
            "models": [],
            "direction": "forward",
            "baseline_model": None,
            "shadow_percentage": 100.0,
            "judge_model": JUDGE_MODEL,
            "max_turns": 5,
            "max_budget": call["max_budget"],
            "ends_at": "2099-01-01T00:00:00Z",
        }
    )


def stats(call: Input) -> list[JobStat]:
    spend = call["stats_spend"]
    if spend is None:
        return []
    return [{"job_id": JOB_ID, "attempts": 0, "spend": spend}]


def litellm_metadata(call: Input) -> dict[str, JsonValue | UserAPIKeyAuth]:
    metadata: dict[str, JsonValue | UserAPIKeyAuth] = {"user_api_key_hash": KEY_HASH}
    auth = call["auth"]
    if auth is not None:
        metadata["user_api_key_auth"] = UserAPIKeyAuth(
            token=auth["token"],
            spend=auth["spend"],
            max_budget=auth["max_budget"],
            team_id=auth["team_id"],
        )
    return metadata


def request_kwargs(
    metadata: dict[str, JsonValue | UserAPIKeyAuth], payload: dict[str, JsonValue]
) -> RequestKwargs:
    messages: list[JsonValue] = [{"role": "user", "content": "what is 2+2?"}]
    return {
        "standard_logging_object": payload,
        "litellm_params": {"metadata": metadata},
        "messages": messages,
    }


class TeamCache(InMemoryCache):
    def __init__(self, row: LiteLLM_TeamTableCachedObj | None) -> None:
        super().__init__()
        self.row = row

    @override
    def get_cache(self, key: str, **kwargs: JsonValue) -> LiteLLM_TeamTableCachedObj | None:
        del kwargs
        if self.row is None or key != f"team_id:{self.row.team_id}":
            return None
        return self.row


def team_row(team: TeamInput | None) -> LiteLLM_TeamTableCachedObj | None:
    if team is None:
        return None
    return LiteLLM_TeamTableCachedObj(
        team_id=team["team_id"], max_budget=team["max_budget"], spend=team["spend"]
    )


def install(team: TeamInput | None, script: SpendScript) -> None:
    cache = UserApiKeyCache(in_memory_cache=TeamCache(team_row(team)))
    module = ProxyServer(PROXY_MODULE)
    module.get_current_spend = script.current
    module.prisma_client = EmptyPrisma()
    module.proxy_logging_obj = logging_obj
    module.user_api_key_cache = cache
    sys.modules[PROXY_MODULE] = module


def row_json(data: AttemptData) -> dict[str, JsonValue]:
    return {
        "job_id": data["job_id"],
        "request_id": data["request_id"],
        "router_name": data["router_name"],
        "outcome": data["outcome"],
        "tier": data["tier"],
        "real_model": data["real_model"],
        "shadow_model": data["shadow_model"],
        "confidence": money_or_null(data["confidence"]),
        "judge_cost": money(data["judge_cost"]),
        "shadow_cost": money(data["shadow_cost"]),
        "shadow_classifier_cost": money(data["shadow_classifier_cost"]),
        "real_cost": money(data["real_cost"]),
        "real_classifier_cost": money(data["real_classifier_cost"]),
        "real_cache_hit": data["real_cache_hit"],
        "error_present": data["error"] is not None,
    }


async def settle(funnel: Funnel, attempts: AttemptTable, rows: int) -> None:
    for _tick in range(TICKS):
        if funnel.events or len(attempts.created) >= rows:
            return
        await asyncio.sleep(0.01)
    raise RuntimeError("the shadow pipeline wrote no attempt row")
