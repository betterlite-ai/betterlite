from collections.abc import Callable
from typing import cast

import httpx
import litellm
from litellm.llms.azure.azure import AzureChatCompletion
from litellm.llms.azure.common_utils import AzureOpenAIError
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from openai import AsyncAzureOpenAI, AsyncOpenAI, AzureOpenAI, OpenAI
from pydantic import JsonValue
from typing_extensions import override

from .fixture import Fixture, ScriptedError, TokenEndpoint, client_for, scripted_provider
from .types import (
    AsyncChat,
    ErrorSpec,
    Input,
    JsonDump,
    StreamCase,
    StreamKwargs,
    SyncChat,
)
from .views import (
    Recorder,
    azure_error_view,
    collect,
    scripted_error_view,
    stored_headers,
    wrapper_view,
)


class ScriptedChatCompletion(AzureChatCompletion):
    get_error: ErrorSpec | None
    gets: list[JsonValue]

    def __init__(self, get_error: ErrorSpec | None) -> None:
        super().__init__()
        self.get_error = get_error
        self.gets = []

    @override
    def get_azure_openai_client(  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L480-L489
        self,
        api_key: str | None,
        api_base: str | None,
        api_version: str | None = None,
        client: AzureOpenAI | AsyncAzureOpenAI | OpenAI | AsyncOpenAI | None = None,
        litellm_params: dict[str, JsonValue] | None = None,
        _is_async: bool = False,
        model: str | None = None,
    ) -> AzureOpenAI | AsyncAzureOpenAI | OpenAI | AsyncOpenAI | None:
        if self.get_error is not None:
            raise ScriptedError(self.get_error)
        params: dict[str, JsonValue] = {} if litellm_params is None else litellm_params
        self.gets.append(
            {
                "api_version": api_version,
                "api_base": api_base,
                "api_key": api_key,
                "model": model,
                "is_async": _is_async,
                "litellm_params": {
                    "azure_ad_token": params.get("azure_ad_token"),
                    "max_retries": params.get("max_retries"),
                    "timeout": params.get("timeout"),
                },
            }
        )
        return client


def data_of(case: StreamCase) -> dict[str, JsonValue]:
    data = case.get("data")
    if data is None:
        return {}
    return dict(data)


def litellm_params_of(case: StreamCase) -> dict[str, JsonValue]:
    params = case.get("litellm_params")
    if params is None:
        return {}
    return params


def stream_kwargs(
    case: StreamCase,
    recorder: Recorder,
    client: AzureOpenAI | AsyncAzureOpenAI | OpenAI | AsyncOpenAI | None,
) -> StreamKwargs:
    provider: Callable[[], str] | None = None
    if "azure_ad_token_provider" in case:
        provider = scripted_provider
    return StreamKwargs(
        logging_obj=recorder,
        api_base=case["api_base"],
        api_key=case.get("api_key"),
        api_version=case["api_version"],
        dynamic_params=bool(case.get("dynamic_params")),
        data=data_of(case),
        model=case["model"],
        timeout=case.get("timeout"),
        max_retries=case["max_retries"],
        azure_ad_token=case.get("azure_ad_token"),
        azure_ad_token_provider=provider,
        client=client,
        litellm_params=litellm_params_of(case),
    )


def call_view(
    handler: ScriptedChatCompletion,
    recorder: Recorder,
    fixture: Fixture,
    endpoint: TokenEndpoint,
    outcome: str,
    wrapper: JsonValue,
    chunks: JsonValue,
    error: JsonValue,
) -> JsonValue:
    return {
        "outcome": outcome,
        "gets": handler.gets,
        "requests": fixture.requests,
        "exchanges": endpoint.forms,
        "pre": recorder.pre,
        "response_headers": stored_headers(recorder),
        "wrapper": wrapper,
        "chunks": chunks,
        "error": error,
    }


async def stream_call(function: str, case: StreamCase) -> JsonValue:
    send = case.get("send")
    fixture = Fixture({} if send is None else send)
    exchange = case.get("exchange")
    endpoint = TokenEndpoint({} if exchange is None else exchange)
    litellm.module_level_client = HTTPHandler(
        client=httpx.Client(transport=httpx.MockTransport(endpoint))
    )
    client = client_for(case, fixture)
    recorder = Recorder()
    handler = ScriptedChatCompletion(case.get("get_error"))
    kwargs = stream_kwargs(case, recorder, client)
    sync_handler = cast(SyncChat, cast(object, handler))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/azure.py#L500-L575
    async_handler = cast(AsyncChat, cast(object, handler))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/azure.py#L577-L652
    asynchronous = function == "async_streaming"
    result: JsonDump
    try:
        if asynchronous:
            result = await async_handler.async_streaming(**kwargs)
        else:
            result = sync_handler.streaming(**kwargs)
    except ScriptedError as error:
        return call_view(
            handler,
            recorder,
            fixture,
            endpoint,
            "error",
            None,
            None,
            scripted_error_view(error),
        )
    except AzureOpenAIError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L46
        return call_view(
            handler,
            recorder,
            fixture,
            endpoint,
            "error",
            None,
            None,
            azure_error_view(error),
        )
    wrapper = wrapper_view(result)
    chunks = await collect(result, asynchronous=asynchronous)
    return call_view(
        handler,
        recorder,
        fixture,
        endpoint,
        "stream",
        wrapper,
        chunks,
        None,
    )


async def run_cases(input_value: Input) -> JsonValue:
    calls: list[JsonValue] = []
    for case in input_value["cases"]:
        calls.append(await stream_call(input_value["function"], case))
    return {"calls": calls}
