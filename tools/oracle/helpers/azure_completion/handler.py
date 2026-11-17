from typing import Final, cast

from litellm.llms.azure.completion.handler import AzureTextCompletion
from litellm.types.utils import ModelResponse
from openai import AsyncAzureOpenAI, AzureOpenAI
from pydantic import JsonValue

from .fixture import Recorder
from .types import (
    API_VERSION,
    Answer,
    AsyncTextCompletion,
    CallCase,
    CompletionKwargs,
    StreamKwargs,
    SyncTextCompletion,
    Turn,
)

SYNC_HANDLER: Final[SyncTextCompletion] = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L31-L51
    SyncTextCompletion, AzureTextCompletion()
)
ASYNC_HANDLER: Final[AsyncTextCompletion] = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L188-L202
    AsyncTextCompletion, AzureTextCompletion()
)


def api_version_of(case: CallCase) -> str:
    version = case.get("api_version")
    if version is None:
        return API_VERSION
    return version


def case_answer(case: CallCase) -> Answer:
    answer = case.get("answer")
    if answer is None:
        return {}
    return answer


def optional_params_of(case: CallCase) -> dict[str, JsonValue]:
    params = case.get("optional_params")
    if params is None:
        return {}
    return dict(params)


def litellm_params_of(case: CallCase) -> dict[str, JsonValue]:
    params = case.get("litellm_params")
    if params is None:
        return {}
    return params


def data_of(case: CallCase) -> dict[str, JsonValue]:
    data = case.get("data")
    if data is None:
        return {}
    return dict(data)


def completion_kwargs(
    case: CallCase, recorder: Recorder, client: AzureOpenAI | AsyncAzureOpenAI
) -> CompletionKwargs:
    return {
        "model": cast(str, case.get("model")),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L31-L51
        "messages": cast(list[Turn], case.get("messages")),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L31-L51
        "model_response": ModelResponse(),
        "api_key": case.get("api_key"),
        "api_base": cast(str, case.get("api_base")),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L31-L51
        "api_version": api_version_of(case),
        "api_type": "azure",
        "azure_ad_token": case.get("azure_ad_token"),
        "azure_ad_token_provider": None,
        "print_verbose": print,
        "timeout": case.get("timeout"),
        "logging_obj": recorder,
        "optional_params": optional_params_of(case),
        "litellm_params": litellm_params_of(case),
        "logger_fn": None,
        "acompletion": bool(case.get("acompletion")),
        "headers": case.get("headers"),
        "client": client,
    }


def stream_kwargs(
    case: CallCase, recorder: Recorder, client: AzureOpenAI | AsyncAzureOpenAI
) -> StreamKwargs:
    return StreamKwargs(
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
