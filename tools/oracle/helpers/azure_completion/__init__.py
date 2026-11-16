import asyncio
from typing import cast

import litellm
from litellm.llms.azure.common_utils import AzureOpenAIError
from litellm.llms.azure.completion.transformation import AzureOpenAITextConfig
from litellm.types.utils import ModelResponse
from pydantic import JsonValue

from .clients import client_for
from .fixture import Fixture, Recorder
from .handler import (
    ASYNC_HANDLER,
    SYNC_HANDLER,
    api_version_of,
    case_answer,
    data_of,
    litellm_params_of,
    optional_params_of,
)
from .types import JSON, REPLY, AsyncStream, AuthCase, CallCase, Input, JsonDump, SyncStream, Turn
from .views import async_chunks, error_view, reply_view, report, sync_chunks

litellm.suppress_debug_info = True


def sync_completion(case: CallCase) -> JsonValue:
    fixture = Fixture(case_answer(case))
    recorder = Recorder()
    client = client_for(case, fixture)
    optional = optional_params_of(case)
    streamed = optional.get("stream") is True
    try:
        result = SYNC_HANDLER.completion(
            model=cast(str, case.get("model")),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L31-L51
            messages=cast(list[Turn], case.get("messages")),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L31-L51
            model_response=ModelResponse(),
            api_key=case.get("api_key"),
            api_base=cast(str, case.get("api_base")),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L31-L51
            api_version=api_version_of(case),
            api_type="azure",
            azure_ad_token=case.get("azure_ad_token"),
            azure_ad_token_provider=None,
            print_verbose=print,
            timeout=case.get("timeout"),
            logging_obj=recorder,
            optional_params=optional,
            litellm_params=litellm_params_of(case),
            logger_fn=None,
            acompletion=False,
            headers=case.get("headers"),
            client=client,
        )
    except AzureOpenAIError as error:
        return report(fixture, recorder, "error", None, None, error_view(error))
    if streamed:
        chunks = sync_chunks(cast(SyncStream, result))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L114-L125
        return report(fixture, recorder, "stream", None, chunks, None)
    reply = REPLY.validate_json(cast(JsonDump, result).model_dump_json())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L174-L177
    return report(fixture, recorder, "buffered", reply_view(reply), None, None)


async def async_completion(case: CallCase) -> JsonValue:
    fixture = Fixture(case_answer(case))
    recorder = Recorder()
    client = client_for(case, fixture)
    optional = optional_params_of(case)
    streamed = bool(optional.get("stream"))
    try:
        result = await ASYNC_HANDLER.completion(
            model=cast(str, case.get("model")),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L188-L202
            messages=cast(list[Turn], case.get("messages")),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L188-L202
            model_response=ModelResponse(),
            api_key=case.get("api_key"),
            api_base=cast(str, case.get("api_base")),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L188-L202
            api_version=api_version_of(case),
            api_type="azure",
            azure_ad_token=case.get("azure_ad_token"),
            azure_ad_token_provider=None,
            print_verbose=print,
            timeout=case.get("timeout"),
            logging_obj=recorder,
            optional_params=optional,
            litellm_params=litellm_params_of(case),
            logger_fn=None,
            acompletion=True,
            headers=case.get("headers"),
            client=client,
        )
    except AzureOpenAIError as error:
        return report(fixture, recorder, "error", None, None, error_view(error))
    if streamed:
        chunks = await async_chunks(cast(AsyncStream, result))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L85-L98
        return report(fixture, recorder, "stream", None, chunks, None)
    reply = REPLY.validate_json(cast(JsonDump, result).model_dump_json())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L235-L238
    return report(fixture, recorder, "buffered", reply_view(reply), None, None)


def sync_stream_call(case: CallCase) -> JsonValue:
    fixture = Fixture(case_answer(case))
    recorder = Recorder()
    client = client_for(case, fixture)
    try:
        stream = SYNC_HANDLER.streaming(
            logging_obj=recorder,
            api_base=cast(str, case.get("api_base")),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L249-L261
            api_key=case.get("api_key"),
            api_version=api_version_of(case),
            data=data_of(case),
            model=cast(str, case.get("model")),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L249-L261
            timeout=case.get("timeout"),
            azure_ad_token=case.get("azure_ad_token"),
            client=client,
            litellm_params=litellm_params_of(case),
        )
    except AzureOpenAIError as error:
        return report(fixture, recorder, "error", None, None, error_view(error))
    return report(fixture, recorder, "stream", None, sync_chunks(stream), None)


async def async_stream_call(case: CallCase) -> JsonValue:
    fixture = Fixture(case_answer(case))
    recorder = Recorder()
    client = client_for(case, fixture)
    try:
        stream = await ASYNC_HANDLER.async_streaming(
            logging_obj=recorder,
            api_base=cast(str, case.get("api_base")),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L302-L314
            api_key=case.get("api_key"),
            api_version=api_version_of(case),
            data=data_of(case),
            model=cast(str, case.get("model")),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L302-L314
            timeout=case.get("timeout"),
            azure_ad_token=case.get("azure_ad_token"),
            client=client,
            litellm_params=litellm_params_of(case),
        )
    except AzureOpenAIError as error:
        return report(fixture, recorder, "error", None, None, error_view(error))
    return report(fixture, recorder, "stream", None, await async_chunks(stream), None)


def completion_call(case: CallCase) -> JsonValue:
    if case.get("acompletion"):
        return asyncio.run(async_completion(case))
    return sync_completion(case)


def stream_call(case: CallCase) -> JsonValue:
    if case.get("acompletion"):
        return asyncio.run(async_stream_call(case))
    return sync_stream_call(case)


def auth_case(case: AuthCase) -> JsonValue:
    headers = SYNC_HANDLER.validate_environment(
        api_key=case["api_key"], azure_ad_token=case["azure_ad_token"]
    )
    return {"headers": JSON.validate_python(headers)}


def config_fields(config: AzureOpenAITextConfig) -> JsonValue:
    return JSON.validate_python(
        {
            "frequency_penalty": config.frequency_penalty,
            "logit_bias": config.logit_bias,
            "max_tokens": config.max_tokens,
            "n": config.n,
            "presence_penalty": config.presence_penalty,
            "stop": config.stop,
            "temperature": config.temperature,
            "top_p": config.top_p,
        }
    )


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "completion":
        return {"calls": [completion_call(case) for case in input_value["cases"]]}
    if input_value["function"] == "stream":
        return {"calls": [stream_call(case) for case in input_value["cases"]]}
    if input_value["function"] == "auth":
        return {"results": [auth_case(case) for case in input_value["cases"]]}
    config = AzureOpenAITextConfig(**input_value["constructor"])
    return {"fields": config_fields(config)}
