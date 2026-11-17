import asyncio

import httpx
import litellm
from litellm.llms.bedrock.common_utils import BedrockError
from pydantic import JsonValue

from .views import chunk_payload, provider_error, raised_error
from .wire import (
    CONVERSE,
    INVOKE_ASYNC,
    INVOKE_SYNC,
    AsyncChunks,
    AsyncClient,
    Case,
    Consume,
    Consumed,
    Fixture,
    Input,
    JSON,
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
        failures.append(JSON.validate_python({"status": error.status_code}))
    return values, failures

def open_sync(case: Case, client: SyncClient) -> OpenedSync:
    if case["path"] == "converse_sync":
        chunks, headers = CONVERSE.make_sync_call(
            client=client,
            api_base=case["api_base"],
            headers=case["headers"],
            data=case["data"],
            model=case["model"],
            messages=case["messages"],
            logging_obj=client.recording,
            json_mode=case["json_mode"],
            fake_stream=case["fake_stream"],
        )
        return chunks, headers
    chunks, headers = INVOKE_SYNC.make_sync_call(
        client=client,
        api_base=case["api_base"],
        headers=case["headers"],
        data=case["data"],
        signed_json_body=None,
        model=case["model"],
        messages=case["messages"],
        logging_obj=client.recording,
        fake_stream=case["fake_stream"],
        json_mode=case["json_mode"],
        bedrock_invoke_provider=case["provider"],
    )
    return chunks, headers

async def run_case_async(case: Case) -> JsonValue:
    client: Fixture
    if case["path"] == "invoke_async":
        client = AsyncClient(case)
        try:
            chunks, headers = await INVOKE_ASYNC.make_call(
                client=client,
                api_base=case["api_base"],
                headers=case["headers"],
                data=case["data"],
                model=case["model"],
                messages=case["messages"],
                logging_obj=client.recording,
                fake_stream=case["fake_stream"],
                json_mode=case["json_mode"],
                bedrock_invoke_provider=case["provider"],
            )
        except BedrockError as error:
            return client.result(None, [], [], provider_error(error, case))
        except httpx.TimeoutException:
            return client.result(None, [], [], raised_error("timeout"))
        except httpx.HTTPError:
            return client.result(None, [], [], raised_error("other"))
    else:
        client = SyncClient(case)
        try:
            chunks, headers = open_sync(case, client)
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
    return JSON.validate_python({"cases": views})
