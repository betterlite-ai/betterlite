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
    options: dict[str, JsonValue] = {
        "max_results": max_results,
        "search_domain_filter": JSON.validate_python(search_domain_filter),
        "max_tokens_per_page": max_tokens_per_page,
        "country": country,
    }
    return {name: option for name, option in options.items() if option is not None}


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
    options = _build_search_optional_params(
        max_results, search_domain_filter, max_tokens_per_page, country
    )
    return {
        **options,
        "query": JSON.validate_python(query),
        "search_provider": search_provider,
        "timeout_seconds": timeout,
        # filter_out_litellm_params: the payload drops its own names, the native side takes the rest
        "kwargs": {
            name: value
            for name, value in kwargs.items()
            if not is_dropped(name, value)
        },
        "api_key": api_key,
        "api_base": api_base,
        "extra_headers": extra_headers,
    }


# The native call and its exception family: the two twins differ only in the wait.
def _search_blocking(payload: CallPayload) -> str:
    session = NativeSearchSession("{}")
    try:
        return session.search(payload.text, payload.blobs)
    except NativeError as error:
        raise_native_error(error)


async def _search_awaiting(payload: CallPayload) -> str:
    session = NativeSearchSession("{}")
    try:
        return await session.asearch(payload.text, payload.blobs)
    except NativeError as error:
        raise_native_error(error)


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
    payload = to_call_payload(_search_values(
        query, search_provider, max_results, search_domain_filter,
        max_tokens_per_page, country, api_key, api_base, timeout, extra_headers, kwargs,
    ))
    return _search_answer(_search_blocking(payload))


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
    return _search_answer(await _search_awaiting(to_call_payload(_search_values(
        query, search_provider, max_results, search_domain_filter,
        max_tokens_per_page, country, api_key, api_base, timeout, extra_headers, kwargs,
    ))))
