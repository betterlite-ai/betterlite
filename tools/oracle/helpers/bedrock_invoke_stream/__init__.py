import asyncio
from unittest import mock

import litellm
from litellm.exceptions import MidStreamFallbackError
from litellm.litellm_core_utils.streaming_handler import CustomStreamWrapper
from litellm.llms.bedrock.chat.invoke_transformations import base_invoke_transformation
from pydantic import JsonValue

from .views import chunk_content, identity_payload, transport_payload
from .wire import (
    JSON,
    WRAPPERS,
    AsyncRecorder,
    Input,
    LoggingRecorder,
    MockReply,
    Run,
    SyncRecorder,
    async_identity_client,
    async_transport_client,
    control_params,
    signed_body,
    sync_identity_client,
    sync_transport_client,
)

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True


# Both wrappers pass the same call to B691; async omits the signed body and keeps the client.
async def async_transport(run: Run) -> JsonValue:
    recorder = AsyncRecorder()
    client = async_transport_client(run)
    with mock.patch.object(  # ast-grep-ignore: no-string-mocking
        base_invoke_transformation, "make_call", recorder
    ):
        _ = await WRAPPERS.get_async_custom_stream_wrapper(
            model=run["model"],
            custom_llm_provider="openai",
            logging_obj=LoggingRecorder(),
            api_base=run["api_base"],
            headers=run["headers"],
            data=run["data"],
            messages=run["messages"],
            client=client,
            json_mode=run["json_mode"],
            signed_json_body=signed_body(run),
            litellm_params=control_params(run["stream_chunk_size"]),
        )
    return transport_payload(recorder)


def sync_transport(run: Run) -> JsonValue:
    supplied = sync_transport_client(run)
    recorder = SyncRecorder(supplied)
    with mock.patch.object(  # ast-grep-ignore: no-string-mocking
        base_invoke_transformation, "make_sync_call", recorder
    ):
        _ = WRAPPERS.get_sync_custom_stream_wrapper(
            model=run["model"],
            custom_llm_provider="openai",
            logging_obj=LoggingRecorder(),
            api_base=run["api_base"],
            headers=run["headers"],
            data=run["data"],
            messages=run["messages"],
            client=supplied,
            json_mode=run["json_mode"],
            signed_json_body=signed_body(run),
            litellm_params=control_params(run["stream_chunk_size"]),
        )
    return transport_payload(recorder)


async def transport_case(run: Run) -> JsonValue:
    if run["wrapper"] == "async":
        return await async_transport(run)
    return sync_transport(run)


async def async_identity(
    run: Run, logging: LoggingRecorder, reply: MockReply
) -> CustomStreamWrapper:
    return await WRAPPERS.get_async_custom_stream_wrapper(
        model=run["model"],
        custom_llm_provider="openai",
        logging_obj=logging,
        api_base=run["api_base"],
        headers=run["headers"],
        data=run["data"],
        messages=run["messages"],
        client=async_identity_client(run, reply),
        json_mode=run["json_mode"],
        signed_json_body=signed_body(run),
        litellm_params=control_params(run["stream_chunk_size"]),
    )


def sync_identity(run: Run, logging: LoggingRecorder, reply: MockReply) -> CustomStreamWrapper:
    return WRAPPERS.get_sync_custom_stream_wrapper(
        model=run["model"],
        custom_llm_provider="openai",
        logging_obj=logging,
        api_base=run["api_base"],
        headers=run["headers"],
        data=run["data"],
        messages=run["messages"],
        client=sync_identity_client(run, reply),
        json_mode=run["json_mode"],
        signed_json_body=signed_body(run),
        litellm_params=control_params(run["stream_chunk_size"]),
    )


# The caller's read loop: the text the stream carried and whether it failed, the pair the case compares.
async def drain_async(wrapper: CustomStreamWrapper) -> tuple[str, bool]:
    text = ""
    try:
        async for chunk in wrapper:
            text += chunk_content(chunk)
    except MidStreamFallbackError:
        return text, True
    return text, False


def drain_sync(wrapper: CustomStreamWrapper) -> tuple[str, bool]:
    text = ""
    try:
        for chunk in wrapper:
            text += chunk_content(chunk)
    except MidStreamFallbackError:
        return text, True
    return text, False


# A real B691 call and B004 wrapper, so the POST, the log entry, the stream and the identity are LiteLLM's.
async def identity_case(run: Run) -> JsonValue:
    logging = LoggingRecorder()
    reply = MockReply(
        run["response_headers"],
        bytes.fromhex(run["response_body"]),
        fails=run["response_fails"],
    )
    if run["wrapper"] == "async":
        wrapper = await async_identity(run, logging, reply)
        text, failed = await drain_async(wrapper)
    else:
        wrapper = sync_identity(run, logging, reply)
        text, failed = drain_sync(wrapper)
    return identity_payload(wrapper, logging, reply, text=text, failed=failed)


async def run_all(input_value: Input) -> list[JsonValue]:
    if input_value["scenario"] == "transport_arguments":
        return [await transport_case(run) for run in input_value["runs"]]
    return [await identity_case(run) for run in input_value["runs"]]


def run(input_value: Input) -> JsonValue:
    views = asyncio.run(run_all(input_value))
    payload: dict[str, JsonValue] = {"cases": views}
    return JSON.validate_python(payload)
