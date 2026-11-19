from collections.abc import Callable
from datetime import UTC, datetime

from litellm.litellm_core_utils.litellm_logging import Logging
from openai import AsyncAzureOpenAI, AsyncOpenAI, AzureOpenAI, OpenAI
from pydantic import JsonValue
from typing_extensions import override

from .fakes import ScriptedError, scripted_provider
from .records import (
    JSON,
    MODEL,
    PRE_ARGS,
    StreamCase,
    StreamKwargs,
)
from .reports import header_map


def pre_view(params: dict[str, JsonValue]) -> JsonValue:
    additional = PRE_ARGS.validate_python(params.get("additional_args"))
    return {
        "input": params.get("input"),
        "api_key": params.get("api_key"),
        "headers": additional.headers,
        "api_base": additional.api_base,
        "acompletion": additional.acompletion,
        "complete_input_dict": additional.complete_input_dict,
    }


class Recorder(Logging):
    def __init__(self) -> None:
        super().__init__(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L576
            model=MODEL,
            messages=None,
            stream=True,
            call_type="completion",
            start_time=datetime(2024, 1, 2, tzinfo=UTC),
            litellm_call_id="call",
            function_id="function",
        )
        self.pre: list[JsonValue] = []

    @override
    def pre_call(self, *args: JsonValue, **params: JsonValue) -> None:
        del args
        self.pre.append(pre_view(params))


def stored_headers(recorder: Recorder) -> JsonValue:
    details: dict[str, JsonValue] = recorder.model_call_details
    return JSON.validate_python(details.get("response_headers"))


def scripted_status(error: ScriptedError) -> tuple[JsonValue, bool]:
    try:
        value: JsonValue = error.status_code
    except AttributeError:
        return None, False
    else:
        return value, True


def scripted_message(error: ScriptedError) -> tuple[JsonValue, bool]:
    try:
        value: JsonValue = error.message
    except AttributeError:
        return None, False
    else:
        return value, True


def scripted_headers(error: ScriptedError) -> JsonValue:
    try:
        value = header_map(error.headers)
    except AttributeError:
        return None
    else:
        return value


def scripted_body(error: ScriptedError) -> JsonValue:
    try:
        value = JSON.validate_python(error.body)
    except AttributeError:
        return None
    else:
        return value


def scripted_error_view(error: ScriptedError) -> JsonValue:
    status, status_present = scripted_status(error)
    message, message_present = scripted_message(error)
    return {
        "class": type(error).__name__,
        "status_present": status_present,
        "status": status,
        "message_present": message_present,
        "message": message,
        "headers": scripted_headers(error),
        "body": scripted_body(error),
    }


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
