import asyncio
from typing import Literal

import litellm
import openai
from litellm.types.llms import openai as litellm_types
from litellm.types.llms.openai import Thread
from openai.types.beta.thread_create_params import Message
from pydantic import BaseModel, ConfigDict, Field, SkipValidation, TypeAdapter
from pydantic import JsonValue as Json

from openai_chat import project_failure

JSON: TypeAdapter[Json] = TypeAdapter(Json)


class ThreadRecord(BaseModel):
    model_config = ConfigDict(extra="forbid")

    id: str
    created_at: int
    metadata: Json
    object: str


class ThreadValue(BaseModel):
    model_config = ConfigDict(extra="forbid")

    thread: ThreadRecord | None


class AssistantThreadsValue(BaseModel):
    model_config = ConfigDict(extra="forbid")

    value: ThreadValue


class AssistantThreadsResult(BaseModel):
    model_config = ConfigDict(extra="forbid")

    assistant_threads: AssistantThreadsValue


class AssistantThreadsCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    operation: Literal["create", "get"]
    mode: Literal["sync", "async"]
    provider: Literal["openai", "azure"]
    thread_id: str | None = None
    messages: SkipValidation[list[Message] | None] = None
    metadata: dict[str, Json] | None = None
    tool_resources: litellm_types.OpenAICreateThreadParamsToolResources | None = None
    kwargs: dict[str, Json] = Field(default_factory=dict)


def create_sync(call: AssistantThreadsCall, api_base: str) -> Thread:
    return litellm.create_thread(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/assistants/main.py#L557
        custom_llm_provider=call.provider,
        messages=call.messages,
        metadata=call.metadata,
        tool_resources=call.tool_resources,
        client=None,
        api_base=api_base,
        **call.kwargs,
    )


async def create_async(call: AssistantThreadsCall, api_base: str) -> Thread:
    return await litellm.acreate_thread(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/assistants/main.py#L526
        custom_llm_provider=call.provider,
        messages=call.messages,
        metadata=call.metadata,
        tool_resources=call.tool_resources,
        api_base=api_base,
        **call.kwargs,
    )


def thread_id_of(call: AssistantThreadsCall) -> str:
    if call.thread_id is None:
        raise ValueError("a get call needs a thread_id")
    return call.thread_id


def get_sync(call: AssistantThreadsCall, api_base: str) -> Thread:
    return litellm.get_thread(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/assistants/main.py#L729
        custom_llm_provider=call.provider,
        thread_id=thread_id_of(call),
        api_base=api_base,
        **call.kwargs,
    )


async def get_async(call: AssistantThreadsCall, api_base: str) -> Thread:
    thread_id = thread_id_of(call)
    return await litellm.aget_thread(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/assistants/main.py#L693
        custom_llm_provider=call.provider,
        thread_id=thread_id,
        client=None,
        api_base=api_base,
        **call.kwargs,
    )


def call_thread(call: AssistantThreadsCall, api_base: str) -> Thread:
    if call.operation == "create":
        if call.mode == "sync":
            return create_sync(call, api_base)
        return asyncio.run(create_async(call, api_base))
    if call.mode == "sync":
        return get_sync(call, api_base)
    return asyncio.run(get_async(call, api_base))


def record(thread: Thread) -> ThreadRecord:
    return ThreadRecord(
        id=thread.id,
        created_at=thread.created_at,
        metadata=JSON.validate_python(thread.metadata),
        object=thread.object,
    )


def run_threads(call: AssistantThreadsCall, api_base: str) -> str:
    try:
        thread = call_thread(call, api_base)
    except openai.APIError as error:
        return project_failure(error).model_dump_json(
            by_alias=True, exclude_unset=True
        )
    result = AssistantThreadsResult(
        assistant_threads=AssistantThreadsValue(
            value=ThreadValue(thread=record(thread))
        )
    )
    return result.model_dump_json(by_alias=True)


class AssistantThreadsRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    assistant_threads: AssistantThreadsCall

    def serialize(self, api_base: str) -> str:
        return run_threads(self.assistant_threads, api_base)
