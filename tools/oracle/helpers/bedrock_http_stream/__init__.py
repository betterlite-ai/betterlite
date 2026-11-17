import asyncio
from collections.abc import Awaitable

import httpx
import litellm
from litellm.llms.bedrock.common_utils import BedrockError
from pydantic import JsonValue

from .views import chunk_payload, failure_status, provider_error, raised_error
from .wire import (
    CONVERSE,
    INVOKE_ASYNC,
    INVOKE_SYNC,
    JSON,
    AsyncChunks,
    AsyncClient,
    Case,
    Consume,
    Consumed,
    Fixture,
    Input,
    OpenArguments,
    OpenedSync,
    SyncChunks,
    SyncClient,
)

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True

async def gather(chunks: SyncChunks | AsyncChunks, consume: Consume) -> Consumed:
    values: list[JsonValue] = []
    failures: list[JsonValue] = []
    if consume == "none":
        return values, failures
    try:
        match chunks:
            case SyncChunks():
                for value in chunks:
                    values.append(chunk_payload(value))
                    if consume == "first":
                        break
            case AsyncChunks():
                async for value in chunks:
                    values.append(chunk_payload(value))
                    if consume == "first":
                        break
    except BedrockError as error:
        failures.append(failure_status(error))
    return values, failures

def open_arguments(case: Case, client: SyncClient | AsyncClient) -> OpenArguments:
    return OpenArguments(
        api_base=case["api_base"],
        headers=case["headers"],
        data=case["data"],
        model=case["model"],
        messages=case["messages"],
        logging_obj=client.recording,
        json_mode=case["json_mode"],
        fake_stream=case["fake_stream"],
    )

async def open_sync(case: Case, client: SyncClient) -> OpenedSync:
    if case["path"] == "converse_sync":
        return CONVERSE.make_sync_call(client=client, **open_arguments(case, client))
    return INVOKE_SYNC.make_sync_call(
        client=client,
        signed_json_body=None,
        bedrock_invoke_provider=case["provider"],
        **open_arguments(case, client),
    )

async def run_case_async(case: Case) -> JsonValue:
    client: Fixture
    pending: Awaitable[tuple[SyncChunks | AsyncChunks, httpx.Headers]]
    if case["path"] == "invoke_async":
        client = AsyncClient(case)
        pending = INVOKE_ASYNC.make_call(
            client=client,
            bedrock_invoke_provider=case["provider"],
            **open_arguments(case, client),
        )
    else:
        client = SyncClient(case)
        pending = open_sync(case, client)
    try:
        chunks, headers = await pending
    except BedrockError as error:
        return client.result(None, [], [], provider_error(error, case))
    except httpx.TimeoutException:
        return client.result(None, [], [], raised_error("timeout"))
    except httpx.HTTPError:
        return client.result(None, [], [], raised_error("other"))
    values, failures = await gather(chunks, case["consume"])
    return client.result(headers, values, failures, None)

def run(input_value: Input) -> JsonValue:
    views = [asyncio.run(run_case_async(case)) for case in input_value["cases"]]
    payload: dict[str, JsonValue] = {"cases": views}
    return JSON.validate_python(payload)
