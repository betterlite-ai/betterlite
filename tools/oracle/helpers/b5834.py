import os
from collections.abc import Callable
from functools import partial
from typing import Final, Literal, Protocol, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.search.transformation import SearchResponse
from litellm.llms.perplexity.decisions.transformation import PerplexityDecisionsConfig
from litellm.llms.perplexity.embedding.transformation import PerplexityEmbeddingConfig
from litellm.llms.perplexity.search.transformation import PerplexitySearchConfig
from litellm.types.llms.openai import ResponsesAPIResponse
from litellm.types.router import GenericLiteLLMParams as Params
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from openai.types.responses.response_input_param import ResponseInputParam
from pydantic import JsonValue, TypeAdapter

from b6412 import (
    EmbeddingCall,
    EmbeddingConfig,
    HeaderCall,
    ReplyCall,
    Urls,
    auth_headers,
    embedding_reply,
    embedding_request,
    response_of,
)
from xai_files import LOG, guarded

Headers: TypeAlias = dict[str, str]
Object: TypeAlias = dict[str, JsonValue]
Query: TypeAlias = str | list[str]
Supported: TypeAlias = Callable[[str], list[str]]
ResponsesHeaders: TypeAlias = Callable[[Headers, str, Params | None], Headers]
BaseUrl: TypeAlias = Callable[[str | None, Object], str]
ResponsesBody: TypeAlias = Callable[
    [str, str | ResponseInputParam, Object, Params, Headers], Object
]
ResponsesReply: TypeAlias = Callable[[str, httpx.Response, Logging], ResponsesAPIResponse]
SearchHeaders: TypeAlias = Callable[[Headers, str | None, str | None], Headers]
SearchBody: TypeAlias = Callable[[Query, Object], Object]
SearchReply: TypeAlias = Callable[[httpx.Response, Logging], SearchResponse]
EMPTY: Final[Object] = {}


class Embedding(EmbeddingConfig, Urls, Protocol):
    @property
    def get_supported_openai_params(self) -> Supported: ...

    @property
    def get_error_class(self) -> Callable[[str, int, Headers], BaseLLMException]: ...


class Responses(Protocol):
    @property
    def get_supported_openai_params(self) -> Supported: ...

    @property
    def validate_environment(self) -> ResponsesHeaders: ...

    @property
    def get_complete_url(self) -> BaseUrl: ...

    @property
    def transform_responses_api_request(self) -> ResponsesBody: ...  # ast-grep-ignore

    @property
    def transform_response_api_response(self) -> ResponsesReply: ...  # ast-grep-ignore

    @property
    def supports_native_websocket(self) -> Callable[[], bool]: ...


class Search(Protocol):
    @property
    def validate_environment(self) -> SearchHeaders: ...

    @property
    def get_complete_url(self) -> BaseUrl: ...

    @property
    def transform_search_request(self) -> SearchBody: ...

    @property
    def transform_search_response(self) -> SearchReply: ...


class BaseCall(TypedDict):
    api_base: str | None
    env: dict[str, str]


class ErrorCall(TypedDict):
    message: str
    status: int


class ResponsesCall(TypedDict):
    model: str
    input: str | ResponseInputParam
    params: Object


class SearchKeyCall(TypedDict):
    headers: Headers
    api_key: str | None
    api_base: str | None
    env: dict[str, str]


class SearchCall(TypedDict):
    query: Query
    optional_params: Object


class EmbeddingInput(TypedDict):
    op: Literal["embedding"]
    urls: list[str | None]
    headers: list[HeaderCall]
    requests: list[EmbeddingCall]
    replies: list[ReplyCall]
    errors: list[ErrorCall]


class ResponsesInput(TypedDict):
    op: Literal["responses"]
    headers: list[HeaderCall]
    urls: list[BaseCall]
    requests: list[ResponsesCall]
    replies: list[ReplyCall]


class SearchInput(TypedDict):
    op: Literal["search"]
    headers: list[SearchKeyCall]
    urls: list[BaseCall]
    requests: list[SearchCall]
    replies: list[ReplyCall]


class DecisionsInput(TypedDict):
    op: Literal["decisions"]
    bases: list[str]


Input: TypeAlias = EmbeddingInput | ResponsesInput | SearchInput | DecisionsInput


class ResultView(TypedDict):
    title: str
    url: str
    snippet: str
    date: str | None
    last_updated: str | None


class SearchView(TypedDict):
    results: list[ResultView]
    object: str


EMBEDDING: Final[Embedding] = PerplexityEmbeddingConfig()
FOUND: Final = ProviderConfigManager.get_provider_responses_api_config(
    provider=LlmProviders.PERPLEXITY
)
if FOUND is None:
    raise ValueError("no Perplexity responses config")
RESPONSES: Final[Responses] = FOUND
SEARCH: Final[Search] = PerplexitySearchConfig()
DECISIONS: Final = PerplexityDecisionsConfig()
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
SEARCH_VIEW: Final[TypeAdapter[SearchView]] = TypeAdapter(SearchView)


def error_view(config: Embedding, call: ErrorCall) -> JsonValue:
    error = config.get_error_class(call["message"], call["status"], {})
    return {
        "status": error.status_code,
        "message": error.message,
        "method": error.request.method,
        "url": str(error.request.url),
        "response_status": error.response.status_code,
    }


def responses_headers(config: Responses, call: HeaderCall) -> JsonValue:
    os.environ.clear()
    os.environ.update(call["env"])
    headers = config.validate_environment(
        dict(call["headers"]), "sonar", Params(api_key=call["api_key"])
    )
    return {"ok": True, "headers": JSON.validate_python(headers)}


def base_url(config: Responses | Search, call: BaseCall) -> JsonValue:
    os.environ.clear()
    os.environ.update(call["env"])
    return config.get_complete_url(call["api_base"], EMPTY)


def responses_request(config: Responses, call: ResponsesCall) -> JsonValue:
    body = config.transform_responses_api_request(
        call["model"], call["input"], dict(call["params"]), Params(), {}
    )
    return {"ok": True, "value": body}


def responses_reply(config: Responses, call: ReplyCall) -> JsonValue:
    reply = config.transform_response_api_response("sonar", response_of(call), LOG)
    return {"ok": True, "id": reply.id}


def search_headers(config: Search, call: SearchKeyCall) -> JsonValue:
    os.environ.clear()
    os.environ.update(call["env"])
    headers = config.validate_environment(dict(call["headers"]), call["api_key"], call["api_base"])
    return {"ok": True, "headers": JSON.validate_python(headers)}


def search_reply(config: Search, call: ReplyCall) -> JsonValue:
    reply = config.transform_search_response(response_of(call), LOG)
    view = SEARCH_VIEW.validate_json(reply.model_dump_json())
    return {"ok": True, "value": JSON.validate_python(view)}


def embedding(config: Embedding, input_value: EmbeddingInput) -> JsonValue:
    urls: list[JsonValue] = [
        config.get_complete_url(base, None, "pplx-embed-v1-0.6b", EMPTY, EMPTY)
        for base in input_value["urls"]
    ]
    headers = partial(auth_headers, config)
    requests = partial(embedding_request, config)
    replies = partial(embedding_reply, config)
    return {
        "supported": JSON.validate_python(config.get_supported_openai_params("m")),
        "urls": urls,
        "headers": [guarded(headers, item) for item in input_value["headers"]],
        "requests": [guarded(requests, item) for item in input_value["requests"]],
        "replies": [guarded(replies, item) for item in input_value["replies"]],
        "errors": [error_view(config, item) for item in input_value["errors"]],
    }


def responses(config: Responses, input_value: ResponsesInput) -> JsonValue:
    supported = config.get_supported_openai_params("sonar")
    answer: Object = {
        "supported": JSON.validate_python(supported),
        "websocket": config.supports_native_websocket(),
    }
    answer["headers"] = [
        guarded(partial(responses_headers, config), item) for item in input_value["headers"]
    ]
    answer["urls"] = [base_url(config, item) for item in input_value["urls"]]
    answer["requests"] = [
        guarded(partial(responses_request, config), item) for item in input_value["requests"]
    ]
    answer["replies"] = [
        guarded(partial(responses_reply, config), item) for item in input_value["replies"]
    ]
    return answer


def search(config: Search, input_value: SearchInput) -> JsonValue:
    headers = partial(search_headers, config)
    replies = partial(search_reply, config)
    return {
        "headers": [guarded(headers, item) for item in input_value["headers"]],
        "urls": [base_url(config, item) for item in input_value["urls"]],
        "requests": [
            config.transform_search_request(item["query"], dict(item["optional_params"]))
            for item in input_value["requests"]
        ],
        "replies": [guarded(replies, item) for item in input_value["replies"]],
    }


def decisions(bases: list[str]) -> JsonValue:
    urls: list[JsonValue] = [DECISIONS.get_complete_url(base, "pplx") for base in bases]
    key_names: list[JsonValue] = list(DECISIONS.api_key_env)
    base_names: list[JsonValue] = list(DECISIONS.api_base_env)
    return {
        "path": DECISIONS.path,
        "api_key_env": key_names,
        "api_base_env": base_names,
        "default_api_base": DECISIONS.get_default_api_base(),
        "urls": urls,
    }


def run(input_value: Input) -> JsonValue:
    if input_value["op"] == "embedding":
        return embedding(EMBEDDING, input_value)
    if input_value["op"] == "responses":
        return responses(RESPONSES, input_value)
    if input_value["op"] == "search":
        return search(SEARCH, input_value)
    return decisions(input_value["bases"])
