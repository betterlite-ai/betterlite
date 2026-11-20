from collections.abc import AsyncIterator, Callable, Coroutine, Iterator
from typing import Final, Protocol, TypeVar

from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.google_genai.transformation import (
    BaseGoogleGenAIGenerateContentConfig as GenAiConfig,
)
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import JsonValue, TypeAdapter

from .case import Case

PROVIDERS: Final[dict[str, LlmProviders]] = {
    "gemini": LlmProviders.GEMINI,
    "vertex_ai": LlmProviders.VERTEX_AI,
}
EXTRA: Final = TypeAdapter(dict[str, object] | None)
Answer = TypeVar("Answer")


class Generated(Protocol):
    def model_dump_json(self) -> str: ...


class SyncEvents(Protocol):
    def __iter__(self) -> Iterator[bytes]: ...


class AsyncEvents(Protocol):
    def __aiter__(self) -> AsyncIterator[bytes]: ...


HANDLER: Final = BaseLLMHTTPHandler()
UNARY: Callable[..., Generated] = HANDLER.generate_content_handler  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L11577-L11707
STREAM: Callable[..., SyncEvents] = HANDLER.generate_content_handler  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L11665-L11669
ASYNC_UNARY: Callable[..., Coroutine[JsonValue, JsonValue, Generated]] = HANDLER.generate_content_handler  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L11709-L11822
ASYNC_STREAM: Callable[..., Coroutine[JsonValue, JsonValue, AsyncEvents]] = HANDLER.generate_content_handler  # noqa: E501  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L11778-L11786


def config_of(case: Case) -> GenAiConfig:
    selected = ProviderConfigManager.get_provider_google_genai_generate_content_config(
        case["model"], PROVIDERS[case["provider"]]
    )
    if selected is None:
        raise ValueError(f"no native GenAI config for {case['model']}")
    return selected


def params_of(case: Case) -> GenericLiteLLMParams:
    params = case["litellm_params"]
    return GenericLiteLLMParams(
        api_key=params.get("api_key"),
        api_base=params.get("api_base"),
        model_info=params.get("model_info"),
    )


def invoke(
    call: Callable[..., Answer],
    case: Case,
    logging_obj: Logging,
    client: HTTPHandler | AsyncHTTPHandler,
    *,
    is_async: bool,
) -> Answer:
    return call(
        model=case["model"],
        contents=case["contents"],
        generate_content_provider_config=config_of(case),
        generate_content_config_dict=case["config"],
        tools=case["tools"],
        custom_llm_provider=case["provider"],
        litellm_params=params_of(case),
        logging_obj=logging_obj,
        extra_headers=EXTRA.validate_python(case["extra_headers"]),
        extra_body=EXTRA.validate_python(case["extra_body"]),
        timeout=case["timeout"],
        _is_async=is_async,
        client=client,
        stream=case["stream"],
        system_instruction=case["system_instruction"],
    )
