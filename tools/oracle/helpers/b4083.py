import asyncio
from datetime import datetime
from decimal import Decimal
from typing import Literal, TypedDict

import litellm
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.integrations.shadow_eval_logger import ActiveShadowEvalJob, ShadowEvalLogger
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.db.shadow_eval_funnel import ShadowEvalFunnelStage
from litellm.proxy.utils import PrismaClient
from litellm.router import Router
from litellm.types.utils import Choices, Message, ModelResponse
from pydantic import BaseModel, ConfigDict, Field, JsonValue

litellm.suppress_debug_info = True

JOBS_CACHE_SIZE = 4
JOBS_CACHE_TTL = 10.0
REAL_MODEL = "claude-opus"
SHADOW_MODEL = "gpt-4o-mini"
JUDGE_MODEL = "judge-model"
USER_MESSAGE = "what is 2+2?"
PROMPT_TOKENS = 1
COMPLETION_TOKENS = 1


class JobRecord(BaseModel):
    id: str
    target_type: str
    target_id: str
    stopped_at: datetime | None = None
    router_name: str
    router_names: tuple[str, ...] = ()
    models: frozenset[str] = frozenset()
    direction: str = "forward"
    baseline_model: str | None = None
    shadow_percentage: float
    judge_model: str
    max_turns: int
    max_budget: float | None = None
    ends_at: datetime


class JobStat(TypedDict):
    job_id: str
    attempts: int
    spend: str


class EndsAfter(TypedDict):
    gt: datetime


class JobsWhere(TypedDict):
    stopped_at: None
    ends_at: EndsAfter


class AttemptData(TypedDict):
    job_id: str
    request_id: str
    router_name: str | None
    outcome: str
    tier: str | None
    real_model: str | None
    shadow_model: str | None
    confidence: float | None
    judge_cost: float
    shadow_cost: float
    shadow_classifier_cost: float
    real_cost: float
    real_classifier_cost: float
    real_cache_hit: bool
    error: str | None


class Price(TypedDict):
    input_cost_per_token: float
    output_cost_per_token: float


class JobIdFilter(BaseModel):
    in_ids: list[str] = Field(alias="in")


class GroupByWhere(BaseModel):
    job_id: JobIdFilter


class GroupByArgs(BaseModel):
    model_config = ConfigDict(populate_by_name=True)

    by: list[str]
    count: bool
    sum_costs: dict[str, bool] = Field(alias="sum")
    where: GroupByWhere


class Input(TypedDict):
    """One case's call input. Every case carries the whole shape; each operation reads
    only its own fields. The attempt cases also carry `job` and `row`: `job` is the row
    the eval pipeline samples, and `row` is the attempt as the pipeline hands it to the
    recorder, before an empty real model is normalized or the error text truncated."""

    operation: Literal["validate_job", "load_jobs", "record_attempt"]
    arm_router: str
    rows: list[dict[str, JsonValue]]
    stats: list[JobStat]
    metadata: dict[str, JsonValue]
    model_group: str
    aliases: dict[str, str]
    hook_calls: int
    job: dict[str, JsonValue]
    row: dict[str, JsonValue]
    request_id: str
    real_text: str
    shadow_reply: str
    judge_reply: str
    shadow_price: Price
    judge_price: Price
    real_cost: float
    shadow_error: str | None


def money(value: float) -> str:
    exact = Decimal(repr(value))
    normalized = exact.normalize()
    return format(normalized, "f")


def money_or_null(value: float | None) -> str | None:
    if value is None:
        return None
    return money(value)


def record_row(data: AttemptData) -> dict[str, JsonValue]:
    error_text = data["error"] or ""
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
        "error_chars": len(error_text),
    }


class JobTable:
    def __init__(self, records: list[JobRecord]) -> None:
        self.records = records
        self.find_count = 0

    async def find_many(self, *, where: JobsWhere) -> list[JobRecord]:
        self.find_count += 1
        cutoff = where["ends_at"]["gt"]
        live = [
            record
            for record in self.records
            if record.stopped_at is None and record.ends_at > cutoff
        ]
        return live


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
            rows.append(
                {
                    "job_id": stat["job_id"],
                    "_count": {"_all": stat["attempts"]},
                    "_sum": {
                        "judge_cost": float(Decimal(stat["spend"])),
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


class Counter:
    def __init__(self, events: list[str]) -> None:
        self.events = events

    async def read(self, _key: str, fallback: float, _max_budget: float) -> float:
        return fallback

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
    return Router(model_list=[], model_group_alias=dict(aliases))


def mock_reply(content: str) -> ModelResponse:
    usage = {
        "prompt_tokens": PROMPT_TOKENS,
        "completion_tokens": COMPLETION_TOKENS,
        "total_tokens": PROMPT_TOKENS + COMPLETION_TOKENS,
    }
    return ModelResponse(
        model=SHADOW_MODEL,
        choices=[
            Choices(
                index=0,
                finish_reason="stop",
                message=Message(role="assistant", content=content),
            )
        ],
        usage=usage,
    )


def deployment(model_name: str, reply: ModelResponse | Exception, price: Price) -> dict[str, object]:
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
    aliases = dict(call["aliases"])
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
    return {
        "standard_logging_object": payload,
        "litellm_params": {"metadata": metadata},
        "messages": [{"role": "user", "content": USER_MESSAGE}],
    }


async def settle(attempts: AttemptTable) -> None:
    for _tick in range(2000):
        if attempts.created:
            return
        await asyncio.sleep(0)
    raise RuntimeError("the shadow pipeline wrote no attempt row")


def empty_response() -> dict[str, JsonValue]:
    return {}


def real_response(text: str) -> dict[str, JsonValue]:
    return {
        "choices": [
            {
                "index": 0,
                "finish_reason": "stop",
                "message": {"role": "assistant", "content": text},
            }
        ],
    }


async def run_load(call: Input) -> JsonValue:
    records = [JobRecord.model_validate(row) for row in call["rows"]]
    jobs = JobTable(records)
    events: list[str] = []
    attempts = AttemptTable(call["stats"], events)
    client = FakePrismaClient(jobs, attempts)
    counter = Counter(events)
    funnel = Funnel()
    logger = ShadowEvalLogger(
        router_provider=RouterProvider(aliased_router(call["aliases"])),
        prisma_provider=PrismaProvider(client),
        jobs_cache=InMemoryCache(max_size_in_memory=JOBS_CACHE_SIZE, default_ttl=JOBS_CACHE_TTL),
        job_spend_reader=counter.read,
        job_spend_writer=counter.write,
        funnel_recorder=funnel,
    )
    payload = hook_payload(call["metadata"], call["model_group"], "req-1", 0.0)
    kwargs = hook_kwargs(call["metadata"], payload)
    for _hook in range(call["hook_calls"]):
        await logger.async_log_success_event(kwargs, empty_response(), None, None)
    group_call = attempts.group_calls[0]
    return {
        "find_calls": jobs.find_count,
        "group_by": group_call.by,
        "group_by_count": group_call.count,
        "group_by_sum": sorted(group_call.sum_costs),
        "stats_ids": group_call.where.job_id.in_ids,
        "funnel": [[job_id, stage] for job_id, stage in funnel.events],
        "attempts": len(attempts.created),
        "spend_writes": counter.events,
    }


async def run_attempt(call: Input) -> JsonValue:
    job_record = JobRecord.model_validate(call["job"])
    arm_target = job_record.baseline_model or job_record.router_name
    jobs = JobTable([job_record])
    events: list[str] = []
    attempts = AttemptTable([], events)
    client = FakePrismaClient(jobs, attempts)
    counter = Counter(events)
    funnel = Funnel()
    logger = ShadowEvalLogger(
        router_provider=RouterProvider(attempt_router(call, arm_target)),
        prisma_provider=PrismaProvider(client),
        jobs_cache=InMemoryCache(max_size_in_memory=JOBS_CACHE_SIZE, default_ttl=JOBS_CACHE_TTL),
        job_spend_reader=counter.read,
        job_spend_writer=counter.write,
        funnel_recorder=funnel,
    )
    payload = hook_payload(
        call["metadata"], call["model_group"], call["request_id"], call["real_cost"]
    )
    kwargs = hook_kwargs(call["metadata"], payload)
    await logger.async_log_success_event(kwargs, real_response(call["real_text"]), None, None)
    await settle(attempts)
    recorded = record_row(attempts.created[0])
    return {
        "events": counter.events,
        "row": recorded,
        "funnel": [[job_id, stage] for job_id, stage in funnel.events],
    }


def run_validate(call: Input) -> JsonValue:
    jobs: list[JsonValue] = []
    for row in call["rows"]:
        try:
            job = ActiveShadowEvalJob.model_validate(row)
        except ValueError:
            jobs.append(
                {
                    "id": row["id"],
                    "valid": False,
                    "direction": None,
                    "arm_router_names": [],
                    "arm_target": None,
                }
            )
            continue
        jobs.append(
            {
                "id": job.id,
                "valid": True,
                "direction": job.direction,
                "arm_router_names": list(job.arm_router_names),
                "arm_target": job.arm_target(call["arm_router"]),
            }
        )
    return {"jobs": jobs}


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "validate_job":
        return run_validate(input_value)
    if input_value["operation"] == "load_jobs":
        return asyncio.run(run_load(input_value))
    return asyncio.run(run_attempt(input_value))
