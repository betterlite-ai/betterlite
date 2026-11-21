from collections.abc import AsyncIterator, Coroutine, Iterator
from typing import Final, Literal, Protocol, TypedDict, TypeVar, cast

import httpx
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

PROVIDERS: Final[dict[str, LlmProviders]] = {
    "gemini": LlmProviders.GEMINI,
    "vertex_ai": LlmProviders.VERTEX_AI,
}
EXTRA: Final = TypeAdapter(dict[str, JsonValue] | None)
Answer_co = TypeVar("Answer_co", covariant=True)

Provider = Literal["gemini", "vertex_ai"]


class Reply(TypedDict):
    status: int
    headers: dict[str, str]
    body: str


class Params(TypedDict, total=False):
    api_key: str
    api_base: str
    model_info: dict[str, JsonValue]


class Case(TypedDict):
    mode: Literal["sync", "async"]
    provider: Provider
    model: str
    contents: JsonValue
    config: dict[str, JsonValue]
    tools: JsonValue
    system_instruction: JsonValue
    litellm_params: Params
    extra_headers: dict[str, str] | None
    extra_body: dict[str, JsonValue] | None
    timeout: float | None
    stream: bool
    reply: Reply


class Generated(Protocol):
    def model_dump_json(self) -> str: ...


class SyncEvents(Protocol):
    def __iter__(self) -> Iterator[bytes]: ...


class AsyncEvents(Protocol):
    def __aiter__(self) -> AsyncIterator[bytes]: ...


class Handler(Protocol[Answer_co]):
    def __call__(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L11577-L11707
        self,
        *,
        model: str,
        contents: JsonValue,
        generate_content_provider_config: GenAiConfig,  # ast-grep-ignore: no-long-names
        generate_content_config_dict: dict[str, JsonValue],
        tools: JsonValue,
        custom_llm_provider: str,
        litellm_params: GenericLiteLLMParams,
        logging_obj: Logging,
        extra_headers: dict[str, JsonValue] | None = None,
        extra_body: dict[str, JsonValue] | None = None,
        timeout: float | httpx.Timeout | None = None,
        _is_async: bool = False,
        client: HTTPHandler | AsyncHTTPHandler | None = None,
        stream: bool = False,
        litellm_metadata: dict[str, JsonValue] | None = None,
        system_instruction: JsonValue | None = None,
    ) -> Answer_co: ...


HANDLER: Final = BaseLLMHTTPHandler()
UNARY: Handler[Generated] = cast(Handler[Generated], HANDLER.generate_content_handler)  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L11577-L11707
STREAM: Handler[SyncEvents] = cast(Handler[SyncEvents], HANDLER.generate_content_handler)  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L11665-L11669
ASYNC_UNARY: Handler[Coroutine[JsonValue, JsonValue, Generated]] = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L11709-L11822
    Handler[Coroutine[JsonValue, JsonValue, Generated]], HANDLER.generate_content_handler  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L11709-L11822
)
ASYNC_STREAM: Handler[Coroutine[JsonValue, JsonValue, AsyncEvents]] = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L11778-L11786
    Handler[Coroutine[JsonValue, JsonValue, AsyncEvents]], HANDLER.generate_content_handler  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L11778-L11786
)


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
    call: Handler[Answer_co],
    case: Case,
    logging_obj: Logging,
    client: HTTPHandler | AsyncHTTPHandler,
    *,
    is_async: bool,
) -> Answer_co:
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
