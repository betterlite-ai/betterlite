import asyncio
import random

import litellm
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.integrations.shadow_eval_logger import ShadowEvalLogger
from pydantic import JsonValue

from b4083.support import (
    JOBS_CACHE_SIZE,
    JOBS_CACHE_TTL,
    AttemptTable,
    Counter,
    FakePrismaClient,
    Funnel,
    JobTable,
    hook_payload,
    real_response,
)

from .support import (
    KEY_HASH,
    REAL_COST,
    REAL_TEXT,
    REQUEST_ID,
    ArmProvider,
    DatabaseProvider,
    Input,
    Reader,
    SpendScript,
    arm_router,
    install,
    job_record,
    litellm_metadata,
    request_kwargs,
    row_json,
    settle,
    stats,
)

litellm.suppress_debug_info = True


async def evaluate(call: Input) -> JsonValue:
    jobs = JobTable([job_record(call)])
    events: list[str] = []
    writes: list[str] = []
    attempts = AttemptTable(stats(call), events)
    client = FakePrismaClient(jobs, attempts)
    writer = Counter(writes)
    reader = Reader(call["job_spend"])
    funnel = Funnel()
    provider = ArmProvider(arm_router(call), pipeline_error=call["pipeline_error"])
    script = SpendScript(call["key_spend"], call["team_spend"])
    if call["auth"] is not None:
        install(call["team"], script)
    metadata = litellm_metadata(call)
    logger = ShadowEvalLogger(
        router_provider=provider,
        prisma_provider=DatabaseProvider(client, load_only=not call["database"]),
        jobs_cache=InMemoryCache(max_size_in_memory=JOBS_CACHE_SIZE, default_ttl=JOBS_CACHE_TTL),
        job_spend_reader=reader.read,
        job_spend_writer=writer.write,
        funnel_recorder=funnel,
    )
    payload_meta: dict[str, JsonValue] = {"user_api_key_hash": KEY_HASH}
    payload = hook_payload(payload_meta, "arm-router", REQUEST_ID, REAL_COST)
    kwargs = request_kwargs(metadata, payload)
    random.seed(1)
    await logger.async_log_success_event(kwargs, real_response(REAL_TEXT), None, None)
    provider.arm_phase = True
    await settle(funnel, attempts, len(call["arms"]))
    rows: list[JsonValue] = [row_json(attempt) for attempt in attempts.created]
    funnel_events: list[JsonValue] = [[job_id, stage] for job_id, stage in funnel.events]
    return {
        "funnel": funnel_events,
        "rows": rows,
        "spend_writes": list[JsonValue](writer.events),
        "advisory_reads": script.reads,
        "job_spend_reads": reader.reads,
    }


def run(input_value: Input) -> JsonValue:
    if not input_value["arms"]:
        raise ValueError("the pipeline needs at least one arm")
    return asyncio.run(evaluate(input_value))
