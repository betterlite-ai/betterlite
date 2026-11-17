import asyncio
import sys
import time
from functools import partial
from typing import cast

import httpx
import litellm
from litellm.llms.anthropic.batches.handler import AnthropicBatchesHandler
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.types.utils import LiteLLMBatch
from pydantic import JsonValue

from .api import BatchesHandlerApi
from .models import REPLY, Retrieve, Scenario
from .wire import (
    LOGGING_MODULE,
    BatchClientCache,
    Capture,
    FakeLoggingModule,
    Recorder,
    apply_environment,
)


def retrieve_once(handler: BatchesHandlerApi, scenario: Scenario) -> LiteLLMBatch:
    if scenario.sync:
        return handler.retrieve_batch(
            _is_async=False,
            batch_id=scenario.batch_id,
            api_base=scenario.api_base,
            api_key=scenario.api_key,
            timeout=9.5,
            max_retries=3,
            logging_obj=None,
        )
    return asyncio.run(
        handler.aretrieve_batch(
            batch_id=scenario.batch_id,
            api_base=scenario.api_base,
            api_key=scenario.api_key,
            timeout=9.5,
            max_retries=3,
            logging_obj=None,
        )
    )


def retrieve_outcome(
    handler: BatchesHandlerApi, scenario: Scenario
) -> tuple[JsonValue, JsonValue]:
    try:
        batch = retrieve_once(handler, scenario)
    except ValueError as error:
        raised: dict[str, JsonValue] = {"kind": "ValueError", "message": str(error)}
        return None, raised
    except httpx.HTTPStatusError as error:
        status: dict[str, JsonValue] = {
            "kind": "status",
            "status": error.response.status_code,
            "message": error.response.text,
        }
        return None, status
    return REPLY.validate_json(batch.model_dump_json()), None


def retrieve_scenario(now: int, scenario: Scenario) -> JsonValue:
    apply_environment(scenario.environment)
    recorder = Recorder(scenario)
    client = AsyncHTTPHandler(transport=httpx.MockTransport(recorder.handle))
    litellm.in_memory_llm_clients_cache = BatchClientCache(client)
    Capture.constructed = None
    Capture.pre = None
    handler = cast(BatchesHandlerApi, AnthropicBatchesHandler())  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/batches/handler.py#L25-L35
    module = sys.modules[LOGGING_MODULE]
    sys.modules[LOGGING_MODULE] = FakeLoggingModule(LOGGING_MODULE)
    clock = time.time
    time.time = partial(float, now)
    try:
        batch, raised = retrieve_outcome(handler, scenario)
    finally:
        time.time = clock
        sys.modules[LOGGING_MODULE] = module
    logging: JsonValue = None
    if Capture.constructed is not None:
        logging = {"constructed": Capture.constructed, "pre_call": Capture.pre}
    return {
        "sync": scenario.sync,
        "requests": recorder.seen,
        "batch": batch,
        "raised": raised,
        "logging": logging,
    }


def retrieve(payload: JsonValue) -> JsonValue:
    case = Retrieve.model_validate(payload)
    scenarios: list[JsonValue] = [
        retrieve_scenario(case.now, scenario) for scenario in case.scenarios
    ]
    return {"scenarios": scenarios}
