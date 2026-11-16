import asyncio
from decimal import Decimal
from typing import Protocol

from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.db.shadow_eval_funnel import ShadowEvalFunnelStage
from litellm.proxy.utils import PrismaClient
from litellm.router import Router
from litellm.types.router import DeploymentTypedDict, RouterModelGroupAliasItem
from litellm.types.utils import Choices, Message, ModelResponse
from pydantic import JsonValue
from typing_extensions import override

from .inputs import AttemptData, GroupByArgs, Input, JobRecord, JobStat, JobsWhere, Price

JOBS_CACHE_SIZE = 4
JOBS_CACHE_TTL = 10
REAL_MODEL = "claude-opus"
SHADOW_MODEL = "gpt-4o-mini"
JUDGE_MODEL = "judge-model"
USER_MESSAGE = "what is 2+2?"
PROMPT_TOKENS = 1
COMPLETION_TOKENS = 1


def money(value: float) -> str:
    exact = Decimal(repr(value))
    normalized = exact.normalize()
    return format(normalized, "f")


def money_or_null(value: float | None) -> str | None:
    if value is None:
        return None
    return money(value)


def record_row(data: AttemptData) -> dict[str, JsonValue]:
    error = data["error"]
    return {
        "request_id": data["request_id"],
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
        "error_chars": 0 if error is None else len(error),
    }


class JobTable:
    def __init__(self, records: list[JobRecord]) -> None:
        self.records = records
        self.find_count = 0

    async def find_many(self, *, where: JobsWhere) -> list[JobRecord]:
        self.find_count += 1
        cutoff = where["ends_at"]["gt"]
        return [
            record
            for record in self.records
            if record.stopped_at is None and record.ends_at > cutoff
        ]


class AttemptTable:
    def __init__(self, stats: list[JobStat], events: list[str]) -> None:
        self.stats = stats
        self.events = events
        self.created: list[AttemptData] = []
        self.group_calls: list[GroupByArgs] = []

    async def group_by(self, **kwargs: JsonValue) -> list[dict[str, JsonValue]]:
        args = GroupByArgs.model_validate(kwargs)
        self.group_calls.append(args)
        rows: list[dict[str, JsonValue]] = []
        for stat in self.stats:
            spend = Decimal(stat["spend"])
            rows.append(
                {
                    "job_id": stat["job_id"],
                    "_count": {"_all": stat["attempts"]},
                    "_sum": {
                        "judge_cost": float(spend),
                        "shadow_cost": 0.0,
                        "shadow_classifier_cost": 0.0,
                    },
                }
            )
        return rows

    async def create(self, *, data: AttemptData) -> None:
        self.events.append(f"insert:{data['request_id']}")
        self.created.append(data)


class FakeDb(PrismaWrapper):
    def __init__(self, jobs: JobTable, attempts: AttemptTable) -> None:
        self.litellm_shadowevaljob = jobs
        self.litellm_shadowevalattempt = attempts


class FakePrismaClient(PrismaClient):
    def __init__(self, jobs: JobTable, attempts: AttemptTable) -> None:
        self.db = FakeDb(jobs, attempts)


class SpendCounter(Protocol):
    async def read(self, key: str, fallback: float, max_budget: float) -> float: ...

    async def write(self, key: str, cost: float) -> None: ...


class Counter(SpendCounter):
    def __init__(self, events: list[str]) -> None:
        self.events = events

    @override
    async def read(self, _key: str, fallback: float, _max_budget: float) -> float:
        return fallback

    @override
    async def write(self, key: str, cost: float) -> None:
        self.events.append(f"write:{key}:{money(cost)}")


class Funnel:
    def __init__(self) -> None:
        self.events: list[tuple[str, ShadowEvalFunnelStage]] = []

    def __call__(self, job_id: str, stage: ShadowEvalFunnelStage) -> None:
        self.events.append((job_id, stage))


class RouterProvider:
    def __init__(self, router: Router) -> None:
        self.router = router

    def __call__(self) -> Router:
        return self.router


class PrismaProvider:
    def __init__(self, client: FakePrismaClient) -> None:
        self.client = client

    def __call__(self) -> FakePrismaClient:
        return self.client


def aliased_router(aliases: dict[str, str]) -> Router:
    if not aliases:
        return Router(model_list=[])
    model_group_alias: dict[str, str | RouterModelGroupAliasItem] = {
        name: target for name, target in aliases.items()
    }
    return Router(model_list=[], model_group_alias=model_group_alias)


def mock_reply(content: str) -> ModelResponse:
    usage = {
        "prompt_tokens": PROMPT_TOKENS,
        "completion_tokens": COMPLETION_TOKENS,
        "total_tokens": PROMPT_TOKENS + COMPLETION_TOKENS,
    }
    choices: list[Choices] = [
        Choices(
            index=0,
            finish_reason="stop",
            message=Message(role="assistant", content=content),
        )
    ]
    return ModelResponse(model=SHADOW_MODEL, choices=choices, usage=usage)


def deployment(
    model_name: str,
    reply: ModelResponse | Exception,
    price: Price,
) -> DeploymentTypedDict:
    return {
        "model_name": model_name,
        "litellm_params": {"model": f"openai/{SHADOW_MODEL}", "mock_response": reply},
        "model_info": dict(price),
    }


def attempt_router(call: Input, arm_target: str) -> Router:
    judge = deployment(JUDGE_MODEL, mock_reply(call["judge_reply"]), call["judge_price"])
    if call["shadow_error"] is None:
        shadow = deployment(arm_target, mock_reply(call["shadow_reply"]), call["shadow_price"])
    else:
        shadow = deployment(arm_target, Exception(call["shadow_error"]), call["shadow_price"])
    aliases: dict[str, str | RouterModelGroupAliasItem] = {
        name: target for name, target in call["aliases"].items()
    }
    if not aliases:
        return Router(model_list=[judge, shadow])
    return Router(model_list=[judge, shadow], model_group_alias=aliases)


def hook_payload(
    metadata: dict[str, JsonValue],
    model_group: str,
    request_id: str,
    real_cost: float,
) -> dict[str, JsonValue]:
    return {
        "id": request_id,
        "call_type": "acompletion",
        "model": REAL_MODEL,
        "model_group": model_group,
        "metadata": metadata,
        "model_parameters": {},
        "response_cost": real_cost,
        "cache_hit": False,
    }


def hook_kwargs(
    metadata: dict[str, JsonValue],
    payload: dict[str, JsonValue],
) -> dict[str, JsonValue]:
    params: dict[str, JsonValue] = {"metadata": metadata}
    messages: list[JsonValue] = [{"role": "user", "content": USER_MESSAGE}]
    return {
        "standard_logging_object": payload,
        "litellm_params": params,
        "messages": messages,
    }


async def settle(attempts: AttemptTable) -> None:
    for _tick in range(500):
        if attempts.created:
            return
        await asyncio.sleep(0.01)
    raise RuntimeError("the shadow pipeline wrote no attempt row")


def real_response(text: str) -> dict[str, JsonValue]:
    message: dict[str, JsonValue] = {"role": "assistant", "content": text}
    return {"choices": [{"index": 0, "finish_reason": "stop", "message": message}]}
