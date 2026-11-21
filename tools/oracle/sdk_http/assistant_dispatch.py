import asyncio
import os
from typing import Literal, Protocol, TypeAlias, cast

import openai
from litellm.router import Router
from litellm.types.router import AssistantsTypedDict
from openai.lib.streaming import AsyncAssistantStreamManager
from openai.pagination import AsyncCursorPage
from openai.types.beta.assistant import Assistant
from openai.types.beta.assistant_deleted import AssistantDeleted
from openai.types.beta.thread import Thread
from openai.types.beta.threads import Message, Run
from pydantic import BaseModel, ConfigDict, Field
from pydantic import JsonValue as Json

from openai_chat import Failure, project_failure

Operation: TypeAlias = Literal[
    "create_assistant",
    "list_assistants",
    "delete_assistant",
    "create_thread",
    "get_thread",
    "create_message",
    "list_messages",
    "run_thread",
]


# The eight wrappers Router.initialize_assistants_endpoint installs, as the factory calls them.
class AssistantCalls(Protocol):
    async def acreate_assistants(self, **kwargs: Json) -> Assistant: ...

    async def aget_assistants(self, **kwargs: Json) -> AsyncCursorPage[Assistant]: ...

    async def adelete_assistant(self, **kwargs: Json) -> AssistantDeleted: ...

    async def acreate_thread(self, **kwargs: Json) -> Thread: ...

    async def aget_thread(self, **kwargs: Json) -> Thread: ...

    async def a_add_message(self, **kwargs: Json) -> Message: ...

    async def aget_messages(self, **kwargs: Json) -> AsyncCursorPage[Message]: ...

    async def arun_thread(
        self, **kwargs: Json
    ) -> Run | AsyncAssistantStreamManager[openai.AsyncAssistantEventHandler]: ...


class Payload(BaseModel):
    model_config = ConfigDict(extra="forbid")

    assistant_id: str | None = None
    model: str | None = None
    name: str | None = None
    role: str | None = None
    content: Json = None
    attachments: Json = None
    messages: Json = None
    metadata: Json = None
    tool_resources: Json = None
    instructions: str | None = None
    additional_instructions: str | None = None
    stream: Json = None
    tools: Json = None
    params: dict[str, Json] = Field(default_factory=dict)


class Config(BaseModel):
    model_config = ConfigDict(extra="forbid")

    custom_llm_provider: str | None = None
    litellm_params: dict[str, Json] | None = None


class OperationCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    operation: Operation
    payload: Payload = Field(default_factory=Payload)
    provider: str | None = None
    thread_id: str | None = None
    assistant_id: str | None = None
    config: Config | None = None
    startup: bool = False


class Call(BaseModel):
    model_config = ConfigDict(extra="forbid")

    runs: list[OperationCall]


class Step(BaseModel):
    model_config = ConfigDict(extra="forbid")

    ids: list[str]
    status: str | None
    events: list[str] | None
    failure: Failure | None


class Outcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    results: list[Step]


class AssistantDispatchValue(BaseModel):
    value: Outcome


class Result(BaseModel):
    assistant_dispatch: AssistantDispatchValue


class AssistantDispatchRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    assistant_dispatch: Call

    def serialize(self, api_base: str) -> str:
        value = Outcome(
            results=[dispatch_run(run, api_base) for run in self.assistant_dispatch.runs]
        )
        return Result(
            assistant_dispatch=AssistantDispatchValue(value=value)
        ).model_dump_json(by_alias=True, exclude_unset=True)


# Router stores the mapping as given and the factory later reads both keys as plain dict entries
def assistant_router(call: OperationCall) -> AssistantCalls:
    stored = cast(AssistantsTypedDict | None, assistants_config(call))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/router.py#L820-L822
    return cast(AssistantCalls, Router(model_list=[], assistants_config=stored))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L1899-L1908


def dispatch_run(call: OperationCall, base_url: str) -> Step:
    router = assistant_router(call)
    body = request_data(call, base_url)
    try:
        return asyncio.run(invoke(router, call, body))
    except openai.APIError as error:
        return Step(ids=[], status=None, events=None, failure=project_failure(error).failure)
    except Exception as error:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L7183-L7190
        failure = Failure(cls=type(error).__name__, status=0, message=str(error))
        return Step(ids=[], status=None, events=None, failure=failure)


# one expanded value: the startup block replaces every os.environ/ occurrence in a string
def expand_value(value: Json) -> Json:
    match value:
        case str() if value.startswith("os.environ/"):
            return os.environ.get(value.replace("os.environ/", ""))
        case str() | list() | dict() | bool() | int() | float() | None:
            return value


# The startup block expands litellm_params values and keeps only a truthy mapping.
def assistants_config(run: OperationCall) -> dict[str, Json] | None:
    config = run.config
    if config is None:
        return None
    if not run.startup:
        direct: dict[str, Json] = {}
        if config.custom_llm_provider is not None:
            direct["custom_llm_provider"] = config.custom_llm_provider
        if config.litellm_params is not None:
            direct["litellm_params"] = config.litellm_params
        return direct
    if config.custom_llm_provider is None and config.litellm_params is None:
        return None
    params = config.litellm_params
    if params is None:
        raise KeyError("litellm_params")
    expanded = {name: expand_value(value) for name, value in params.items()}
    startup: dict[str, Json] = {"litellm_params": expanded}
    if config.custom_llm_provider is not None:
        startup["custom_llm_provider"] = config.custom_llm_provider
    return startup


def request_data(run: OperationCall, api_base: str) -> dict[str, Json]:
    payload = run.payload
    data: dict[str, Json] = dict(payload.params)
    fields: list[tuple[str, Json]] = [
        ("assistant_id", payload.assistant_id),
        ("model", payload.model),
        ("name", payload.name),
        ("role", payload.role),
        ("content", payload.content),
        ("attachments", payload.attachments),
        ("messages", payload.messages),
        ("metadata", payload.metadata),
        ("tool_resources", payload.tool_resources),
        ("instructions", payload.instructions),
        ("additional_instructions", payload.additional_instructions),
        ("stream", payload.stream),
        ("tools", payload.tools),
    ]
    data.update({name: value for name, value in fields if value is not None})
    if run.provider is not None:
        data["custom_llm_provider"] = run.provider
    data["api_base"] = api_base
    return data


async def invoke(router: AssistantCalls, run: OperationCall, data: dict[str, Json]) -> Step:
    if run.operation in ("create_assistant", "list_assistants", "delete_assistant"):
        return await assistant_outcome(router, run, data)
    if run.operation in ("create_thread", "get_thread"):
        return await thread_outcome(router, run, data)
    if run.operation in ("create_message", "list_messages"):
        return await message_outcome(router, run, data)
    return await run_outcome(router, run, data)


async def assistant_outcome(
    router: AssistantCalls, run: OperationCall, data: dict[str, Json]
) -> Step:
    if run.operation == "create_assistant":
        assistant = await router.acreate_assistants(**data)
        return Step(ids=[assistant.id], status=None, events=None, failure=None)
    if run.operation == "list_assistants":
        page = await router.aget_assistants(**data)
        return Step(
            ids=[assistant.id for assistant in page.data],
            status=None,
            events=None,
            failure=None,
        )
    deleted = await router.adelete_assistant(assistant_id=run.assistant_id, **data)
    return Step(ids=[deleted.id], status=None, events=None, failure=None)


async def thread_outcome(
    router: AssistantCalls, run: OperationCall, data: dict[str, Json]
) -> Step:
    if run.operation == "create_thread":
        thread = await router.acreate_thread(**data)
    else:
        thread = await router.aget_thread(thread_id=run.thread_id, **data)
    return Step(ids=[thread.id], status=None, events=None, failure=None)


async def message_outcome(
    router: AssistantCalls, run: OperationCall, data: dict[str, Json]
) -> Step:
    if run.operation == "create_message":
        message = await router.a_add_message(thread_id=run.thread_id, **data)
        return Step(ids=[message.id], status=None, events=None, failure=None)
    page = await router.aget_messages(thread_id=run.thread_id, **data)
    return Step(
        ids=[message.id for message in page.data],
        status=None,
        events=None,
        failure=None,
    )


async def run_outcome(
    router: AssistantCalls, run: OperationCall, data: dict[str, Json]
) -> Step:
    answer = await router.arun_thread(thread_id=run.thread_id, **data)
    match answer:
        case Run():
            return Step(ids=[answer.id], status=answer.status, events=None, failure=None)
        case AsyncAssistantStreamManager():
            events = await drain(answer)
            return Step(ids=[], status=None, events=events, failure=None)


async def drain(
    manager: AsyncAssistantStreamManager[openai.AsyncAssistantEventHandler],
) -> list[str]:
    async with manager as stream:
        return [event.event async for event in stream]
