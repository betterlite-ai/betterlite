import asyncio

import litellm
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.integrations.shadow_eval_logger import ActiveShadowEvalJob, ShadowEvalLogger
from pydantic import JsonValue

from .inputs import Input, JobRecord
from .support import (
    JOBS_CACHE_SIZE,
    JOBS_CACHE_TTL,
    AttemptTable,
    Counter,
    FakePrismaClient,
    Funnel,
    JobTable,
    PrismaProvider,
    RouterProvider,
    aliased_router,
    attempt_router,
    hook_kwargs,
    hook_payload,
    real_response,
    record_row,
    settle,
)

litellm.suppress_debug_info = True


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
    empty: dict[str, JsonValue] = {}
    for _hook in range(call["hook_calls"]):
        await logger.async_log_success_event(kwargs, empty, None, None)
    group_call = attempts.group_calls[0]
    return {
        "find_calls": jobs.find_count,
        "group_by": list[JsonValue](group_call.by),
        "group_by_count": group_call.count,
        "group_by_sum": list[JsonValue](sorted(group_call.sum_costs)),
        "stats_ids": list[JsonValue](group_call.where.job_id.in_ids),
        "funnel": [[job_id, stage] for job_id, stage in funnel.events],
        "attempts": len(attempts.created),
        "spend_writes": list[JsonValue](counter.events),
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
        "events": list[JsonValue](counter.events),
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
