import asyncio
from typing import cast

import litellm
from litellm.llms.azure.common_utils import AzureOpenAIError
from litellm.llms.azure.completion.transformation import AzureOpenAITextConfig
from pydantic import JsonValue

from .fixture import Fixture, Recorder, client_for
from .handler import (
    ASYNC_HANDLER,
    SYNC_HANDLER,
    case_answer,
    completion_kwargs,
    stream_kwargs,
)
from .types import JSON, REPLY, AuthCase, CallCase, ConfigView, Input, JsonDump
from .views import error_view, reply_view, report, stream_chunks

litellm.suppress_debug_info = True


async def completion_call(case: CallCase) -> JsonValue:
    fixture = Fixture(case_answer(case))
    recorder = Recorder()
    kwargs = completion_kwargs(case, recorder, client_for(case, fixture))
    asynchronous = kwargs["acompletion"]
    flag = kwargs["optional_params"].get("stream")
    try:
        result: object = (
            await ASYNC_HANDLER.completion(**kwargs)
            if asynchronous
            else SYNC_HANDLER.completion(**kwargs)
        )
    except AzureOpenAIError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L46
        return report(fixture, recorder, "error", error=error_view(error))
    streamed = bool(flag) if asynchronous else flag is True
    if streamed:
        chunks = await stream_chunks(result, asynchronous=asynchronous)
        return report(fixture, recorder, "stream", chunks=chunks)
    reply = REPLY.validate_json(cast(JsonDump, result).model_dump_json())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L174-L177
    return report(fixture, recorder, "buffered", reply=reply_view(reply))


async def stream_call(case: CallCase) -> JsonValue:
    fixture = Fixture(case_answer(case))
    recorder = Recorder()
    kwargs = stream_kwargs(case, recorder, client_for(case, fixture))
    asynchronous = bool(case.get("acompletion"))
    try:
        result: object = (
            await ASYNC_HANDLER.async_streaming(**kwargs)
            if asynchronous
            else SYNC_HANDLER.streaming(**kwargs)
        )
    except AzureOpenAIError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L46
        return report(fixture, recorder, "error", error=error_view(error))
    chunks = await stream_chunks(result, asynchronous=asynchronous)
    return report(fixture, recorder, "stream", chunks=chunks)


def auth_case(case: AuthCase) -> JsonValue:
    headers = SYNC_HANDLER.validate_environment(
        api_key=case["api_key"], azure_ad_token=case["azure_ad_token"]
    )
    return {"headers": JSON.validate_python(headers)}


def config_fields(config: AzureOpenAITextConfig) -> JsonValue:
    view = cast(ConfigView, config)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/transformation.py#L31-L51
    return JSON.validate_python(
        {
            "frequency_penalty": view.frequency_penalty,
            "logit_bias": view.logit_bias,
            "max_tokens": view.max_tokens,
            "n": view.n,
            "presence_penalty": view.presence_penalty,
            "stop": view.stop,
            "temperature": view.temperature,
            "top_p": view.top_p,
        }
    )


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "completion":
        return {"calls": [asyncio.run(completion_call(case)) for case in input_value["cases"]]}
    if input_value["function"] == "stream":
        return {"calls": [asyncio.run(stream_call(case)) for case in input_value["cases"]]}
    if input_value["function"] == "auth":
        return {"results": [auth_case(case) for case in input_value["cases"]]}
    config = AzureOpenAITextConfig(**input_value["constructor"])
    return {"fields": config_fields(config)}
