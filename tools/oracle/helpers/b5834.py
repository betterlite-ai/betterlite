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
from litellm.llms.perplexity.responses.transformation import PerplexityResponsesConfig
from litellm.llms.perplexity.search.transformation import PerplexitySearchConfig
from litellm.types.llms.openai import ResponsesAPIResponse
from litellm.types.router import GenericLiteLLMParams as Params
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
ResponsesBody: TypeAlias = Callable[[str, str | ResponseInputParam, Object, Params, Headers], Object]
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
RESPONSES: Final[Responses] = PerplexityResponsesConfig()
SEARCH: Final[Search] = PerplexitySearchConfig()
DECISIONS: Final = PerplexityDecisionsConfig()
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
SEARCH_VIEW: Final[TypeAdapter[SearchView]] = TypeAdapter(SearchView)


def error_view(call: ErrorCall) -> JsonValue:
    error = EMBEDDING.get_error_class(call["message"], call["status"], {})
    return {
        "status": error.status_code,
        "message": error.message,
        "method": error.request.method,
        "url": str(error.request.url),
        "response_status": error.response.status_code,
    }


def responses_headers(call: HeaderCall) -> JsonValue:
    os.environ.clear()
    os.environ.update(call["env"])
    headers = RESPONSES.validate_environment(
        dict(call["headers"]), "sonar", Params(api_key=call["api_key"])
    )
    return {"ok": True, "headers": JSON.validate_python(headers)}


def base_url(config: Responses | Search, call: BaseCall) -> JsonValue:
    os.environ.clear()
    os.environ.update(call["env"])
    return config.get_complete_url(call["api_base"], EMPTY)


def responses_request(call: ResponsesCall) -> JsonValue:
    body = RESPONSES.transform_responses_api_request(
        call["model"], call["input"], dict(call["params"]), Params(), {}
    )
    return {"ok": True, "value": body}


def responses_reply(call: ReplyCall) -> JsonValue:
    reply = RESPONSES.transform_response_api_response("sonar", response_of(call), LOG)
    return {"ok": True, "id": reply.id}


def search_headers(call: SearchKeyCall) -> JsonValue:
    os.environ.clear()
    os.environ.update(call["env"])
    headers = SEARCH.validate_environment(dict(call["headers"]), call["api_key"], call["api_base"])
    return {"ok": True, "headers": JSON.validate_python(headers)}


def search_request(call: SearchCall) -> JsonValue:
    return SEARCH.transform_search_request(call["query"], dict(call["optional_params"]))


def search_reply(call: ReplyCall) -> JsonValue:
    reply = SEARCH.transform_search_response(response_of(call), LOG)
    view = SEARCH_VIEW.validate_json(reply.model_dump_json())
    return {"ok": True, "value": JSON.validate_python(view)}


def embedding(input_value: EmbeddingInput) -> JsonValue:
    urls: list[JsonValue] = [
        EMBEDDING.get_complete_url(base, None, "pplx-embed-v1-0.6b", EMPTY, EMPTY)
        for base in input_value["urls"]
    ]
    headers = partial(auth_headers, EMBEDDING)
    requests = partial(embedding_request, EMBEDDING)
    replies = partial(embedding_reply, EMBEDDING)
    return {
        "supported": JSON.validate_python(EMBEDDING.get_supported_openai_params("m")),
        "urls": urls,
        "headers": [guarded(headers, item) for item in input_value["headers"]],
        "requests": [guarded(requests, item) for item in input_value["requests"]],
        "replies": [guarded(replies, item) for item in input_value["replies"]],
        "errors": [error_view(item) for item in input_value["errors"]],
    }


def responses(input_value: ResponsesInput) -> JsonValue:
    return {
        "supported": JSON.validate_python(RESPONSES.get_supported_openai_params("sonar")),
        "websocket": RESPONSES.supports_native_websocket(),
        "headers": [guarded(responses_headers, item) for item in input_value["headers"]],
        "urls": [base_url(RESPONSES, item) for item in input_value["urls"]],
        "requests": [guarded(responses_request, item) for item in input_value["requests"]],
        "replies": [guarded(responses_reply, item) for item in input_value["replies"]],
    }


def search(input_value: SearchInput) -> JsonValue:
    return {
        "headers": [guarded(search_headers, item) for item in input_value["headers"]],
        "urls": [base_url(SEARCH, item) for item in input_value["urls"]],
        "requests": [search_request(item) for item in input_value["requests"]],
        "replies": [guarded(search_reply, item) for item in input_value["replies"]],
    }


def decisions(bases: list[str]) -> JsonValue:
    urls: list[JsonValue] = [DECISIONS.get_complete_url(base, "pplx") for base in bases]
    return {
        "path": DECISIONS.path,
        "api_key_env": list(DECISIONS.api_key_env),
        "api_base_env": list(DECISIONS.api_base_env),
        "default_api_base": DECISIONS.get_default_api_base(),
        "urls": urls,
    }


def run(input_value: Input) -> JsonValue:
    if input_value["op"] == "embedding":
        return embedding(input_value)
    if input_value["op"] == "responses":
        return responses(input_value)
    if input_value["op"] == "search":
        return search(input_value)
    return decisions(input_value["bases"])
