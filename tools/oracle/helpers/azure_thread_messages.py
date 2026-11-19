import asyncio
from collections.abc import Coroutine, Generator
from contextlib import contextmanager
from typing import Literal, Protocol, TypedDict, cast

import httpx
import pydantic
from litellm.llms.azure.assistants import AzureAssistantsAPI
from openai import AsyncAzureOpenAI, AzureOpenAI
from pydantic import JsonValue

from azure_assistant_definitions import (
    JSON,
    SdkClient,
    Step as BaseStep,
    Wire,
    client_params,
    report,
    reported,
    session,
    values_of,
)

THREAD = '{"id":"thread_1","object":"thread","created_at":7,"metadata":{"team":"a"}}'
ENDPOINT = "https://supplied.test"
VERSION = "2024-01-01"
KEY = "supplied-key"
TIMEOUT = 42.0

ASSISTANTS = AzureAssistantsAPI()


class SyncThreadCall(Protocol):
    def __call__(
        self, *, client: SdkClient | None, **params: JsonValue
    ) -> pydantic.BaseModel: ...


class AsyncThreadCall(Protocol):
    def __call__(
        self, *, client: SdkClient | None, **params: JsonValue
    ) -> Coroutine[None, None, pydantic.BaseModel]: ...


CREATE_THREAD: SyncThreadCall = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/assistants.py#L497-L553
    SyncThreadCall,
    ASSISTANTS.create_thread,
)
ASYNC_CREATE_THREAD: AsyncThreadCall = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/assistants.py#L425-L457
    AsyncThreadCall,
    ASSISTANTS.create_thread,
)
GET_THREAD: SyncThreadCall = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/assistants.py#L618-L656
    SyncThreadCall,
    ASSISTANTS.get_thread,
)
ASYNC_GET_THREAD: AsyncThreadCall = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/assistants.py#L555-L580
    AsyncThreadCall,
    ASSISTANTS.get_thread,
)


class Step(BaseStep, total=False):
    messages: list[JsonValue]
    metadata: dict[str, JsonValue]
    thread_id: str


class Input(TypedDict, total=False):
    thread_create: list[Step]
    thread_retrieve: list[Step]
    assistants_beta_default: list[Step]
    azure_thread_async_flags: list[Step]


class StepReport(Protocol):
    def __call__(self, step: Step) -> JsonValue: ...


@contextmanager
def sdk_client(
    kind: Literal["sync", "async"], wire: Wire
) -> Generator[SdkClient, None, None]:
    client: SdkClient
    if kind == "async":
        client = AsyncAzureOpenAI(
            api_key=KEY,
            azure_endpoint=ENDPOINT,
            api_version=VERSION,
            timeout=TIMEOUT,
            http_client=httpx.AsyncClient(transport=httpx.MockTransport(wire)),
        )
    else:
        client = AzureOpenAI(
            api_key=KEY,
            azure_endpoint=ENDPOINT,
            api_version=VERSION,
            timeout=TIMEOUT,
            http_client=httpx.Client(transport=httpx.MockTransport(wire)),
        )
    try:
        yield client
    finally:
        close_sdk(client)


def close_sdk(client: SdkClient) -> None:
    match client:
        case AsyncAzureOpenAI():
            asyncio.run(client.close())
        case AzureOpenAI():
            client.close()


@contextmanager
def supplied_sdk(step: Step, wire: Wire) -> Generator[SdkClient | None, None, None]:
    if "supplied" not in step:
        yield None
        return
    with sdk_client(step["supplied"]["kind"], wire) as client:
        yield client


def record_sync(wire: Wire, result: pydantic.BaseModel) -> None:
    wire.kind = "sync"
    wire.result = JSON.validate_json(result.model_dump_json())


def record_async(wire: Wire, result: Coroutine[None, None, pydantic.BaseModel]) -> None:
    with asyncio.Runner() as runner:
        settled = runner.run(result)
    wire.kind = "async"
    wire.result = JSON.validate_json(settled.model_dump_json())


def run_thread(
    step: Step,
    params: dict[str, JsonValue],
    flag: JsonValue,
    sync_call: SyncThreadCall,
    async_call: AsyncThreadCall,
) -> JsonValue:
    with session(step, THREAD) as wire, reported(wire), supplied_sdk(step, wire) as supplied:
        if flag is True:
            record_async(wire, async_call(client=supplied, **params))
        else:
            record_sync(wire, sync_call(client=supplied, **params))
    return report(wire)


def create_step(step: Step) -> JsonValue:
    values = values_of(step)
    params = client_params(values)
    del params["client"]
    messages: JsonValue = None
    if "messages" in step:
        messages = step["messages"]
    metadata: JsonValue = None
    if "metadata" in step:
        metadata = step["metadata"]
    params["messages"] = messages
    params["metadata"] = metadata
    params["acreate_thread"] = values.flag
    return run_thread(step, params, values.flag, CREATE_THREAD, ASYNC_CREATE_THREAD)


def retrieve_step(step: Step) -> JsonValue:
    values = values_of(step)
    params = client_params(values)
    del params["client"]
    params["thread_id"] = step["thread_id"]
    params["aget_thread"] = values.flag
    return run_thread(step, params, values.flag, GET_THREAD, ASYNC_GET_THREAD)


def mixed_step(step: Step) -> JsonValue:
    if "thread_id" in step:
        return retrieve_step(step)
    return create_step(step)


def reports(steps: list[Step], step_report: StepReport) -> JsonValue:
    produced: list[JsonValue] = [step_report(step) for step in steps]
    shown: dict[str, JsonValue] = {"steps": produced}
    return shown


def run(argument: Input) -> JsonValue:
    if "thread_create" in argument:
        return reports(argument["thread_create"], create_step)
    if "thread_retrieve" in argument:
        return reports(argument["thread_retrieve"], retrieve_step)
    if "assistants_beta_default" in argument:
        return reports(argument["assistants_beta_default"], mixed_step)
    if "azure_thread_async_flags" in argument:
        return reports(argument["azure_thread_async_flags"], mixed_step)
    raise ValueError("expected one azure thread messages variant")
