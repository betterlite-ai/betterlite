from collections.abc import Mapping
from typing import Final

from pydantic import BaseModel, ConfigDict, JsonValue, PrivateAttr, TypeAdapter

from betterlite._completion.arguments import TimeoutValues
from betterlite._completion.errors import raise_native_error
from betterlite._endpoints.payload import CallPayload, CallValue, is_dropped, to_call_payload
from betterlite._native import NativeError, NativeSearchSession

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
SEARCH_OBJECT: Final = "search"


class SearchResult(BaseModel):
    model_config = ConfigDict(extra="allow", protected_namespaces=())

    title: str
    url: str
    snippet: str
    date: str | None = None
    last_updated: str | None = None


class SearchResponse(BaseModel):
    model_config = ConfigDict(extra="allow", protected_namespaces=())

    results: list[SearchResult]
    object: str = SEARCH_OBJECT
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)

    def set_hidden_params(self, hidden_params: dict[str, JsonValue]) -> None:
        self._hidden_params = hidden_params


class SearchAnswer(BaseModel):
    response: SearchResponse
    hidden_params: dict[str, JsonValue]


SEARCH_ANSWER: Final[TypeAdapter[SearchAnswer]] = TypeAdapter(SearchAnswer)


# _build_search_optional_params: only the options the caller set reach the provider
def _build_search_optional_params(
    max_results: int | None = None,
    search_domain_filter: list[str] | None = None,
    max_tokens_per_page: int | None = None,
    country: str | None = None,
) -> dict[str, JsonValue]:
    optional_params: dict[str, JsonValue] = {}
    if max_results is not None:
        optional_params["max_results"] = max_results
    if search_domain_filter is not None:
        optional_params["search_domain_filter"] = JSON.validate_python(search_domain_filter)
    if max_tokens_per_page is not None:
        optional_params["max_tokens_per_page"] = max_tokens_per_page
    if country is not None:
        optional_params["country"] = country
    return optional_params


# filter_out_litellm_params starts with the names the payload drops; the rest is the native filter's
def _optional_kwargs(kwargs: Mapping[str, CallValue]) -> dict[str, CallValue]:
    kept: dict[str, CallValue] = {}
    for name, value in kwargs.items():
        if not is_dropped(name, value):
            kept[name] = value
    return kept


def _search_values(
    query: str | list[str],
    search_provider: str,
    max_results: int | None,
    search_domain_filter: list[str] | None,
    max_tokens_per_page: int | None,
    country: str | None,
    api_key: str | None,
    api_base: str | None,
    timeout: float | TimeoutValues | None,
    extra_headers: dict[str, str] | None,
    kwargs: Mapping[str, CallValue],
) -> dict[str, CallValue]:
    values: dict[str, CallValue] = {
        "query": JSON.validate_python(query),
        "search_provider": search_provider,
        "timeout_seconds": timeout,
        "kwargs": _optional_kwargs(kwargs),
    }
    optional_params = _build_search_optional_params(
        max_results, search_domain_filter, max_tokens_per_page, country
    )
    for name, option in optional_params.items():
        values[name] = option
    if api_key is not None:
        values["api_key"] = api_key
    if api_base is not None:
        values["api_base"] = api_base
    if extra_headers is not None:
        values["extra_headers"] = extra_headers
    return values


def _search_answer(text: str) -> SearchResponse:
    envelope = SEARCH_ANSWER.validate_json(text)
    response = envelope.response
    response.set_hidden_params(envelope.hidden_params)
    return response


def search(
    query: str | list[str],
    search_provider: str,
    max_results: int | None = None,
    search_domain_filter: list[str] | None = None,
    max_tokens_per_page: int | None = None,
    country: str | None = None,
    api_key: str | None = None,
    api_base: str | None = None,
    timeout: float | TimeoutValues | None = None,
    extra_headers: dict[str, str] | None = None,
    **kwargs: CallValue,
) -> SearchResponse:
    payload: CallPayload = to_call_payload(
        _search_values(
            query,
            search_provider,
            max_results,
            search_domain_filter,
            max_tokens_per_page,
            country,
            api_key,
            api_base,
            timeout,
            extra_headers,
            kwargs,
        )
    )
    session = NativeSearchSession("{}")
    try:
        answer = session.search(payload.text, payload.blobs)
    except NativeError as error:
        raise_native_error(error)
    return _search_answer(answer)


async def asearch(
    query: str | list[str],
    search_provider: str,
    max_results: int | None = None,
    search_domain_filter: list[str] | None = None,
    max_tokens_per_page: int | None = None,
    country: str | None = None,
    api_key: str | None = None,
    api_base: str | None = None,
    timeout: float | TimeoutValues | None = None,
    extra_headers: dict[str, str] | None = None,
    **kwargs: CallValue,
) -> SearchResponse:
    payload: CallPayload = to_call_payload(
        _search_values(
            query,
            search_provider,
            max_results,
            search_domain_filter,
            max_tokens_per_page,
            country,
            api_key,
            api_base,
            timeout,
            extra_headers,
            kwargs,
        )
    )
    session = NativeSearchSession("{}")
    try:
        answer = await session.asearch(payload.text, payload.blobs)
    except NativeError as error:
        raise_native_error(error)
    return _search_answer(answer)
