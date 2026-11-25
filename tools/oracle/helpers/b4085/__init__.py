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

from .inputs import Input
from .support import (
    KEY_HASH,
    REAL_COST,
    REAL_TEXT,
    REQUEST_ID,
    ArmProvider,
    ClientProvider,
    LoadOnlyProvider,
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


def result(
    funnel: Funnel,
    attempts: AttemptTable,
    writer: Counter,
    reader: Reader,
    script: SpendScript,
) -> JsonValue:
    rows: list[JsonValue] = []
    for attempt in attempts.created:
        rows.append(row_json(attempt))
    events: list[JsonValue] = []
    for job_id, stage in funnel.events:
        pair: list[JsonValue] = [job_id, stage]
        events.append(pair)
    return {
        "funnel": events,
        "rows": rows,
        "spend_writes": list[JsonValue](writer.events),
        "advisory_reads": script.reads,
        "job_spend_reads": reader.reads,
    }


async def evaluate(call: Input) -> JsonValue:
    jobs = JobTable([job_record(call)])
    events: list[str] = []
    writes: list[str] = []
    attempts = AttemptTable(stats(call), events)
    client = FakePrismaClient(jobs, attempts)
    writer = Counter(writes)
    reader = Reader(call["job_spend"])
    funnel = Funnel()
    provider = ArmProvider(arm_router(call), call["pipeline_error"])
    script = SpendScript(call["key_spend"], call["team_spend"])
    if call["auth"] is not None:
        install(call["team"], script)
    metadata = litellm_metadata(call)
    logger = ShadowEvalLogger(
        router_provider=provider,
        prisma_provider=ClientProvider(client) if call["database"] else LoadOnlyProvider(client),
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
    return result(funnel, attempts, writer, reader, script)


def run(input_value: Input) -> JsonValue:
    outcome = asyncio.run(evaluate(input_value))
    return outcome
