import asyncio
import copy
import json
from collections.abc import Awaitable, Callable, Iterator
from contextlib import suppress
from typing import Literal, NotRequired, TypeAlias, TypedDict, cast
from unittest.mock import patch

import litellm
import litellm.llms.codestral.completion.handler as codestral_handler
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.litellm_core_utils.prompt_templates import huggingface_template_handler
from litellm.litellm_core_utils.streaming_handler import CustomStreamWrapper
from litellm.llms.codestral.completion.handler import TextCompletionCodestralError
from litellm.types.utils import TextCompletionResponse

# pyright: reportPrivateUsage=false, reportUnknownMemberType=false, reportUnknownVariableType=false, reportUnknownArgumentType=false

Object: TypeAlias = dict[str, object]


class Settings(TypedDict, total=False):
    suffix: str
    temperature: float
    top_p: float
    max_tokens: int
    min_tokens: int
    stream: bool
    random_seed: int
    stop: str


class PromptTemplate(TypedDict, total=False):
    roles: dict[str, dict[str, str]]
    initial_prompt_value: str
    final_prompt_value: str
    bos_token: str
    eos_token: str


class Call(TypedDict):
    mode: Literal["sync", "async"]
    api_key: str | None
    model: str
    messages: list[Object]
    timeout: float
    headers: NotRequired[dict[str, str]]
    api_base: NotRequired[str]
    custom_prompt_dict: NotRequired[dict[str, PromptTemplate]]
    optional_params: NotRequired[Object]


class Input(TypedDict):
    calls: list[Call]
    settings: NotRequired[Settings]


class TransportStopError(Exception):
    pass


class FixtureLinesResponse:
    def iter_lines(self) -> Iterator[str]:
        return iter(())


class SyncClient:
    def __init__(self) -> None:
        self.calls: list[Object] = []

    def post(
        self,
        url: str | None = None,
        headers: object = None,
        data: str = "",
        stream: object = None,
    ) -> FixtureLinesResponse:
        self.calls.append(request_facts(url, headers, data))
        if stream is True:
            return FixtureLinesResponse()
        raise TransportStopError


class AsyncClient:
    def __init__(self) -> None:
        self.calls: list[Object] = []
        self.timeout: object = None

    async def post(
        self,
        url: str | None = None,
        headers: object = None,
        data: str = "",
        stream: object = None,
    ) -> FixtureLinesResponse:
        del stream
        self.calls.append(request_facts(url, headers, data))
        raise TransportStopError


class AsyncClientFactory:
    def __init__(self) -> None:
        self.client = AsyncClient()

    def __call__(self, llm_provider: object, params: Object) -> AsyncClient:
        del llm_provider
        self.client.timeout = params.get("timeout")
        return self.client


class FixtureLogger:
    def __init__(self) -> None:
        self.events: list[Object] = []
        self.model_call_details: Object = {}

    def pre_call(self, **params: object) -> None:
        additional = cast(Object, params.get("additional_args"))
        self.events.append({
            "phase": "pre_call",
            "input": params.get("input"),
            "api_key": params.get("api_key"),
            "complete_input_dict": copy.deepcopy(additional.get("complete_input_dict")),
            "headers": additional.get("headers"),
            "api_base": additional.get("api_base"),
            "acompletion": additional.get("acompletion"),
        })

    def post_call(self, **params: object) -> None:
        del params


def ignore_verbose(*_args: object) -> None:
    pass


def request_facts(url: str | None, headers: object, data: str) -> Object:
    return {"url": url, "headers": headers, "body": cast(object, json.loads(data))}


def unreachable(hf_model_name: str) -> Object:
    del hf_model_name
    return {"status": "failure"}


class CompletionArguments(TypedDict):
    model: str
    messages: list[Object]
    api_base: str
    custom_prompt_dict: dict[str, PromptTemplate]
    model_response: TextCompletionResponse
    print_verbose: Callable[..., None]
    encoding: None
    api_key: str
    logging_obj: Logging
    optional_params: Object
    timeout: float
    acompletion: bool
    headers: dict[str, str]


def completion_arguments(call: Call, logger: FixtureLogger) -> CompletionArguments:
    return CompletionArguments(
        model=call["model"],
        messages=call["messages"],
        api_base=cast(str, call.get("api_base")),
        custom_prompt_dict=call.get("custom_prompt_dict", {}),
        model_response=TextCompletionResponse(),
        print_verbose=ignore_verbose,
        encoding=None,
        api_key=cast(str, call["api_key"]),
        logging_obj=cast(Logging, logger),
        optional_params=dict(call.get("optional_params", {})),
        timeout=float(call["timeout"]),
        acompletion=call["mode"] == "async",
        headers=call.get("headers", {}),
    )


def sync_call(call: Call, logger: FixtureLogger) -> object:
    handler = litellm.CodestralTextCompletion()
    return cast(object, handler.completion(**completion_arguments(call, logger)))


async def async_call(call: Call, logger: FixtureLogger) -> object:
    handler = litellm.CodestralTextCompletion()
    pending = handler.completion(**completion_arguments(call, logger))
    result = await cast(Awaitable[object], pending)
    if isinstance(result, CustomStreamWrapper):
        with suppress(TransportStopError):
            await cast(Awaitable[object], result.fetch_stream())
    return result


def drive(call: Call, logger: FixtureLogger) -> object:
    if call["mode"] == "sync":
        return sync_call(call, logger)
    return asyncio.run(async_call(call, logger))


def facts(
    call: Call,
    result: object,
    sync_client: SyncClient,
    client: AsyncClient,
    logger: FixtureLogger,
) -> Object:
    provider: object = None
    if isinstance(result, CustomStreamWrapper):
        provider = result.custom_llm_provider
    calls = sync_client.calls or client.calls
    if not calls:
        return {"outcome": "error"}
    return {
        "mode": call["mode"],
        "request": calls[0],
        "provider": provider,
        "timeout": client.timeout,
        "events": logger.events,
    }


def run_call(call: Call) -> Object:
    logger = FixtureLogger()
    sync_client = SyncClient()
    factory = AsyncClientFactory()
    try:
        with (
            patch.object(litellm, "module_level_client", sync_client),
            patch.object(litellm, "module_level_aclient", factory.client),
            patch.object(codestral_handler, "get_async_httpx_client", factory),
        ):
            result = drive(call, logger)
    except TransportStopError:
        result = None
    except (ValueError, TextCompletionCodestralError):
        if not sync_client.calls and not factory.client.calls:
            return {"outcome": "error"}
        result = None
    return facts(call, result, sync_client, factory.client, logger)


def apply_settings(settings: Settings) -> None:
    config_type = litellm.CodestralTextCompletionConfig
    for name, value in settings.items():
        if value is not None:
            setattr(config_type, name, value)


def run(data: Input) -> object:
    apply_settings(data.get("settings", {}))
    with (
        patch.object(huggingface_template_handler, "_get_tokenizer_config", unreachable),
        patch.object(huggingface_template_handler, "_get_chat_template_file", unreachable),
    ):
        return [run_call(call) for call in data["calls"]]
