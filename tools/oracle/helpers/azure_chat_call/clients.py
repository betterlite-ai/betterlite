from collections.abc import Callable

from litellm.llms.azure.azure import AzureChatCompletion
from litellm.llms.azure.chat.o_series_handler import AzureOpenAIO1ChatCompletion
from litellm.types.utils import ModelResponse
from pydantic import JsonValue

from .fixtures import (
    api_base_of,
    api_version_of,
    litellm_params_of,
    optional_params_of,
    timeout_of,
)
from .types import (
    AnyClient,
    AzureClient,
    CallCase,
    CompletionKwargs,
    OSeriesKwargs,
)
from .views import Recorder, forward_view, params_view


def completion_kwargs(
    case: CallCase, recorder: Recorder, client: AzureClient | None
) -> CompletionKwargs:
    return {
        "model": case.get("model"),
        "messages": case.get("messages"),
        "model_response": ModelResponse(),
        "api_key": case.get("api_key"),
        "api_base": api_base_of(case),
        "api_version": api_version_of(case),
        "api_type": "azure",
        "azure_ad_token": case.get("azure_ad_token"),
        "azure_ad_token_provider": None,
        "dynamic_params": False,
        "print_verbose": print,
        "timeout": timeout_of(case),
        "logging_obj": recorder,
        "optional_params": optional_params_of(case),
        "litellm_params": litellm_params_of(case),
        "logger_fn": None,
        "acompletion": bool(case.get("acompletion")),
        "headers": case.get("headers"),
        "client": client,
    }


def o_series_kwargs(
    case: CallCase, recorder: Recorder, client: AzureClient | None
) -> OSeriesKwargs:
    return OSeriesKwargs(
        model_response=ModelResponse(),
        timeout=timeout_of(case),
        optional_params=optional_params_of(case),
        litellm_params=litellm_params_of(case),
        logging_obj=recorder,
        model=case.get("model"),
        messages=case.get("messages"),
        print_verbose=print,
        api_key=case.get("api_key"),
        api_base=api_base_of(case),
        api_version=api_version_of(case),
        dynamic_params=None,
        azure_ad_token=case.get("azure_ad_token"),
        acompletion=bool(case.get("acompletion")),
        logger_fn=None,
        headers=case.get("headers"),
        client=client,
        organization=case.get("organization"),
        custom_llm_provider=None,
        drop_params=case.get("drop_params"),
        shared_session=None,
    )


class ScriptedAzure:
    mock: AzureClient
    gets: list[JsonValue]
    gateways: list[JsonValue]

    # ast-grep-ignore: no-long-names
    def get_azure_openai_client(  # noqa: PLR0913, PLR0917
        self,
        api_key: str | None,
        api_base: str | None,
        api_version: str | None = None,
        client: AnyClient | None = None,
        litellm_params: dict[str, JsonValue] | None = None,
        _is_async: bool = False,  # noqa: FBT001, FBT002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L480-L489
        model: str | None = None,
    ) -> AnyClient | None:
        params = {} if litellm_params is None else litellm_params
        record: dict[str, JsonValue] = {
            "api_version": api_version,
            "api_base": api_base,
            "api_key": api_key,
            "model": model,
            "is_async": _is_async,
            "supplied": client is not None,
        }
        record["litellm_params"] = params_view(params)
        self.gets.append(record)
        if client is None:
            return self.mock
        return client

    # ast-grep-ignore: no-long-names
    def _init_azure_client_for_cloudflare_ai_gateway(  # noqa: PLR0913, PLR0917
        self,
        api_base: str,
        model: str,
        api_version: str,
        max_retries: int,
        timeout: float,
        litellm_params: dict[str, JsonValue],
        api_key: str | None,
        azure_ad_token: str | None,
        azure_ad_token_provider: Callable[[], str] | None,
        acompletion: bool,  # noqa: FBT001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L698-L711
        client: AzureClient | None = None,
    ) -> AzureClient:
        del litellm_params, azure_ad_token, azure_ad_token_provider
        if client is not None:
            return client
        self.gateways.append(
            {
                "api_base": api_base,
                "model": model,
                "api_version": api_version,
                "max_retries": max_retries,
                "timeout": timeout,
                "api_key": api_key,
                "acompletion": acompletion,
            }
        )
        return self.mock


class ScriptedChat(  # type: ignore[misc]  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L698-L711
    ScriptedAzure, AzureChatCompletion
):
    def __init__(self, mock: AzureClient) -> None:
        super().__init__()
        self.mock = mock
        self.gets = []
        self.gateways = []


class ScriptedOSeries(  # type: ignore[misc]  # pyright: ignore[reportIncompatibleMethodOverride]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/common_utils.py#L698-L711
    ScriptedAzure, AzureOpenAIO1ChatCompletion
):
    def __init__(self, mock: AzureClient) -> None:
        super().__init__()
        self.mock = mock
        self.gets = []
        self.gateways = []


class Capture:
    def __init__(self) -> None:
        self.calls: list[JsonValue] = []

    def __call__(self, **params: JsonValue) -> ModelResponse:
        self.calls.append(forward_view(params))
        return ModelResponse()
