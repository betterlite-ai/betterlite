import asyncio
from collections.abc import Coroutine
from typing import Protocol, TypeAlias, TypedDict, cast

from litellm.llms.azure.assistants import AzureAssistantsAPI
from openai import AssistantEventHandler, AsyncAssistantEventHandler
from openai.lib.streaming import AssistantStreamManager, AsyncAssistantStreamManager
from openai.types.beta import AssistantStreamEvent
from openai.types.beta.threads.run import Run
from pydantic import JsonValue
from typing_extensions import override

from azure_assistant_definitions import values_of
from azure_assistant_definitions.sdk import JSON, SdkClient
from azure_assistant_definitions.sdk import Step as SharedStep

ENDPOINT = "https://supplied.test"
KEY = "supplied-key"
TIMEOUT = 42.0
VERSION = "2024-01-01"


class Reply(TypedDict):
    status: int
    body: str
    headers: dict[str, str]


class RunTarget(TypedDict):
    thread_id: str
    assistant_id: str


class RunKeywords(RunTarget, total=False):
    additional_instructions: str | None
    instructions: str | None
    metadata: dict[str, JsonValue] | None
    model: str | None
    tools: list[JsonValue] | None


NO_REPLY: Reply = {
    "status": 500,
    "body": '{"error":{"message":"the fixture queues no reply"}}',
    "headers": {},
}


# The shared step reads the facade arguments and litellm_params; these add the run and the wire.
class Step(SharedStep, total=False):
    arun_thread: bool
    stream: bool
    handler: bool
    replies: list[Reply]


class RunStep(Step):
    run: RunKeywords


class Input(TypedDict, total=False):
    run_poll: list[RunStep]
    run_stream: list[RunStep]
    run_instructions_tools: list[RunStep]
    run_failure: list[RunStep]
    run_dispatch: list[RunStep]
    instructions_tools_overrides: list[RunStep]
    stream_event_handler: list[RunStep]
    assistants_beta_default: list[RunStep]


ASSISTANTS = AzureAssistantsAPI()

SyncManager: TypeAlias = AssistantStreamManager[AssistantEventHandler]
AsyncManager: TypeAlias = AsyncAssistantStreamManager[AsyncAssistantEventHandler]
RunResult: TypeAlias = Run | Coroutine[None, None, Run] | SyncManager | AsyncManager


# LiteLLM types its run facade itself; the adapter passes the keywords its call received.
class RunCall(Protocol):
    def __call__(
        self,
        *,
        client: SdkClient | None,
        event_handler: AssistantEventHandler | AsyncAssistantEventHandler | None,
        **params: JsonValue,
    ) -> RunResult: ...


class Recorder(AssistantEventHandler):
    def __init__(self, calls: list[JsonValue]) -> None:
        super().__init__()
        self.calls = calls

    @override
    def on_event(self, event: AssistantStreamEvent) -> None:
        self.calls.append(event.event)

    @override
    def on_end(self) -> None:
        self.calls.append("end")


class AsyncRecorder(AsyncAssistantEventHandler):
    def __init__(self, calls: list[JsonValue]) -> None:
        super().__init__()
        self.calls = calls

    @override
    async def on_event(self, event: AssistantStreamEvent) -> None:
        self.calls.append(event.event)

    @override
    async def on_end(self) -> None:
        self.calls.append("end")


RUN_THREAD: RunCall = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/assistants.py#L807-L905
    RunCall, ASSISTANTS.run_thread
)


def handler_for(
    step: RunStep, calls: list[JsonValue], *, asynchronous: bool
) -> AssistantEventHandler | AsyncAssistantEventHandler | None:
    if not step.get("handler"):
        return None
    if asynchronous:
        return AsyncRecorder(calls)
    return Recorder(calls)


def dispatched(
    step: RunStep, client: SdkClient | None, calls: list[JsonValue]
) -> dict[str, JsonValue]:
    values = values_of(step)
    run = step["run"]
    arun = step.get("arun_thread")
    streaming = step.get("stream")
    result = RUN_THREAD(
        thread_id=run["thread_id"],
        assistant_id=run["assistant_id"],
        additional_instructions=run.get("additional_instructions"),
        instructions=run.get("instructions"),
        metadata=run.get("metadata"),
        model=run.get("model"),
        stream=streaming,
        tools=run.get("tools"),
        api_key=values.api_key,
        api_base=values.api_base,
        api_version=values.api_version,
        azure_ad_token=values.azure_ad_token,
        timeout=values.timeout,
        max_retries=values.max_retries,
        client=client,
        arun_thread=arun,
        event_handler=handler_for(step, calls, asynchronous=arun is True and streaming is True),
        litellm_params=values.litellm_params,
    )
    if arun is True and streaming is True:
        async_manager = cast(AsyncManager, result)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/assistants.py#L822-L850
        return {"events": asyncio.run(drained(async_manager))}
    if arun is True:
        settled = asyncio.run(cast(Coroutine[None, None, Run], result))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/assistants.py#L836-L838
        return {"kind": "async", "result": JSON.validate_json(settled.model_dump_json())}
    if streaming is True:
        sync_manager = cast(SyncManager, result)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/assistants.py#L890-L901
        return consumed_sync(sync_manager)
    return {"kind": "sync", "result": JSON.validate_json(cast(Run, result).model_dump_json())}  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/assistants.py#L902-L904


def consumed_sync(manager: SyncManager) -> dict[str, JsonValue]:
    with manager as entered:
        events = [JSON.validate_json(event.model_dump_json()) for event in entered]
    return {"events": events}


async def drained(manager: AsyncManager) -> list[JsonValue]:
    async with manager as entered:
        return [JSON.validate_json(event.model_dump_json()) async for event in entered]
