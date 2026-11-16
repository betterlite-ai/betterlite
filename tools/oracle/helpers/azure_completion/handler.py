from collections.abc import Callable
from typing import Final, Protocol, cast

from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure.completion.handler import AzureTextCompletion
from litellm.types.utils import ModelResponse
from openai import AsyncAzureOpenAI, AzureOpenAI
from pydantic import JsonValue

from .types import API_VERSION, Answer, AsyncStream, CallCase, JsonDump, SyncStream, Turn


class SyncTextCompletion(Protocol):
    def validate_environment(
        self, api_key: str | None, azure_ad_token: str | None
    ) -> dict[str, JsonValue]: ...

    def completion(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L31-L51
        self,
        *,
        model: str,
        messages: list[Turn],
        model_response: ModelResponse,
        api_key: str | None,
        api_base: str,
        api_version: str,
        api_type: str,
        azure_ad_token: str | None,
        azure_ad_token_provider: None,
        print_verbose: Callable[..., None],
        timeout: float | None,
        logging_obj: Logging,
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        logger_fn: None,
        acompletion: bool,
        headers: dict[str, str] | None,
        client: AzureOpenAI | AsyncAzureOpenAI,
    ) -> SyncStream | JsonDump: ...

    def streaming(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L249-L261
        self,
        *,
        logging_obj: Logging,
        api_base: str,
        api_key: str | None,
        api_version: str,
        data: dict[str, JsonValue],
        model: str,
        timeout: float | None,
        azure_ad_token: str | None,
        client: AzureOpenAI | AsyncAzureOpenAI,
        litellm_params: dict[str, JsonValue],
    ) -> SyncStream: ...


class AsyncTextCompletion(Protocol):
    async def completion(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L188-L202
        self,
        *,
        model: str,
        messages: list[Turn],
        model_response: ModelResponse,
        api_key: str | None,
        api_base: str,
        api_version: str,
        api_type: str,
        azure_ad_token: str | None,
        azure_ad_token_provider: None,
        print_verbose: Callable[..., None],
        timeout: float | None,
        logging_obj: Logging,
        optional_params: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        logger_fn: None,
        acompletion: bool,
        headers: dict[str, str] | None,
        client: AzureOpenAI | AsyncAzureOpenAI,
    ) -> AsyncStream | JsonDump: ...

    async def async_streaming(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/completion/handler.py#L302-L314
        self,
        *,
        logging_obj: Logging,
        api_base: str,
        api_key: str | None,
        api_version: str,
        data: dict[str, JsonValue],
        model: str,
        timeout: float | None,
        azure_ad_token: str | None,
        client: AzureOpenAI | AsyncAzureOpenAI,
        litellm_params: dict[str, JsonValue],
    ) -> AsyncStream: ...


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
