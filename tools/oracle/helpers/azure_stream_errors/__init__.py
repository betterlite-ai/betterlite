import asyncio
from typing import TypedDict, cast

import litellm
from litellm.llms.azure.common_utils import AzureOpenAIError
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from pydantic import JsonValue

from .fakes import (
    Fixture,
    ScriptedChatCompletion,
    ScriptedError,
    TokenEndpoint,
    client_for,
)
from .records import (
    CHUNK,
    CREATED_FLOOR,
    JSON,
    AsyncChat,
    AsyncStream,
    ChunkView,
    Input,
    JsonDump,
    StreamCase,
    SyncChat,
    SyncStream,
    WrapperView,
)
from .reports import azure_error_view
from .views import (
    Recorder,
    scripted_error_view,
    stored_headers,
    stream_kwargs,
)


def chunk_view(view: ChunkView) -> JsonValue:
    choice = view["choices"][0]
    return {
        "content": choice["delta"].get("content"),
        "role": choice["delta"].get("role"),
        "finish_reason": choice["finish_reason"],
        "index": choice["index"],
        "model": view["model"],
        "created_is_current": view["created"] > CREATED_FLOOR,
    }


def wrapper_view(result: JsonDump) -> JsonValue:
    view = cast(WrapperView, cast(object, result))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/streaming_handler.py#L208-L299
    options = view.stream_options
    if options is None:
        stream_options: JsonValue = None
    else:
        stream_options = {"include_usage": options.get("include_usage") is True}
    return {
        "model": view.model,
        "provider": view.custom_llm_provider,
        "stream_options": stream_options,
        "response_headers": JSON.validate_python(view._response_headers),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/streaming_handler.py#L208-L299
        "additional_headers": JSON.validate_python(view._hidden_params.get("additional_headers")),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/streaming_handler.py#L208-L299
    }


async def collect(result: JsonDump, *, asynchronous: bool) -> list[JsonValue]:
    if asynchronous:
        stream = cast(AsyncStream, cast(object, result))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/streaming_handler.py#L1978-L1990
        return [
            chunk_view(CHUNK.validate_json(chunk.model_dump_json())) async for chunk in stream
        ]
    buffered = cast(SyncStream, cast(object, result))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/streaming_handler.py#L1768-L1775
    return [chunk_view(CHUNK.validate_json(chunk.model_dump_json())) for chunk in buffered]


class CallContext(TypedDict):
    handler: ScriptedChatCompletion
    recorder: Recorder
    fixture: Fixture
    endpoint: TokenEndpoint


def call_view(
    context: CallContext,
    outcome: str,
    wrapper: JsonValue,
    chunks: JsonValue,
    error: JsonValue,
) -> JsonValue:
    return {
        "outcome": outcome,
        "gets": context["handler"].gets,
        "requests": context["fixture"].requests,
        "exchanges": context["endpoint"].forms,
        "pre": context["recorder"].pre,
        "response_headers": stored_headers(context["recorder"]),
        "wrapper": wrapper,
        "chunks": chunks,
        "error": error,
    }


async def stream_call(function: str, case: StreamCase) -> JsonValue:
    send = case.get("send")
    fixture = Fixture({} if send is None else send)
    exchange = case.get("exchange")
    endpoint = TokenEndpoint({} if exchange is None else exchange)
    client = client_for(case, fixture)
    recorder = Recorder()
    handler = ScriptedChatCompletion(case.get("get_error"))
    context = CallContext(
        handler=handler, recorder=recorder, fixture=fixture, endpoint=endpoint
    )
    kwargs = stream_kwargs(case, recorder, client)
    sync_handler = cast(SyncChat, cast(object, handler))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/azure.py#L500-L575
    async_handler = cast(AsyncChat, cast(object, handler))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/azure.py#L577-L652
    asynchronous = function == "async_streaming"
    # get_azure_ad_token_from_oidc posts through litellm.module_level_client
    litellm.module_level_client = cast(HTTPHandler, cast(object, endpoint))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L249
    result: JsonDump
    try:
        if asynchronous:
            result = await async_handler.async_streaming(**kwargs)
        else:
            result = sync_handler.streaming(**kwargs)
    except ScriptedError as error:
        return call_view(context, "error", None, None, scripted_error_view(error))
    except AzureOpenAIError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L46
        return call_view(context, "error", None, None, azure_error_view(error))
    wrapper = wrapper_view(result)
    chunks = await collect(result, asynchronous=asynchronous)
    return call_view(context, "stream", wrapper, chunks, None)


async def run_cases(input_value: Input) -> JsonValue:
    calls = [await stream_call(input_value["function"], case) for case in input_value["cases"]]
    return {"calls": calls}


def run(input_value: Input) -> JsonValue:
    litellm.suppress_debug_info = True
    return asyncio.run(run_cases(input_value))
