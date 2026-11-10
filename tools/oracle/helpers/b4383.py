import json
from collections.abc import Callable
from typing import Protocol, TypeAlias, cast
from unittest.mock import patch

from litellm.caching.qdrant_semantic_cache import QdrantSemanticCache

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

DEFAULT_API_BASE = "http://qdrant.test"


class _CacheOps(Protocol):
    def get_cache(self, key: str, **kwargs: Json) -> object: ...


class _Response:
    def __init__(self, status_code: int, body: Json) -> None:
        self.status_code = status_code
        self.body = body

    @property
    def text(self) -> str:
        return json.dumps(self.body)

    def json(self) -> Json:
        return self.body


class _FakeClient:
    def __init__(self, responses: list[tuple[int, Json]]) -> None:
        self.responses = list(responses)
        self.calls: list[dict[str, Json]] = []

    def get(self, **kwargs: Json) -> _Response:
        return self._answer(kwargs)

    def put(self, **kwargs: Json) -> _Response:
        return self._answer(kwargs)

    def post(self, **kwargs: Json) -> _Response:
        return self._answer(kwargs)

    def _answer(self, kwargs: dict[str, Json]) -> _Response:
        self.calls.append(kwargs)
        status, body = self.responses.pop(0)
        return _Response(status, body)


def _client_factory(client: _FakeClient) -> Callable[..., _FakeClient]:
    def build(**_kwargs: object) -> _FakeClient:
        return client

    return build


def _embedding(vector: list[float]) -> Callable[..., dict[str, Json]]:
    def embed(**_kwargs: object) -> dict[str, Json]:
        return cast(dict[str, Json], {"data": [{"embedding": vector}]})

    return embed


def _text(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _number(value: Json, name: str) -> float:
    if not isinstance(value, float):
        raise ValueError(f"{name} must be a number")
    return value


def _object(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def _array(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def _responses(value: Json) -> list[tuple[int, Json]]:
    rows = _array(value, "responses")
    parsed: list[tuple[int, Json]] = []
    for row in rows:
        fields = _object(row, "response")
        status = fields.get("status")
        if not isinstance(status, int) or isinstance(status, bool):
            raise ValueError("status must be an integer")
        parsed.append((status, fields.get("body")))
    return parsed


def _search_call(client: _FakeClient) -> dict[str, Json] | None:
    for call in client.calls:
        if str(call.get("url", "")).endswith("/points/search"):
            return call
    return None


def run(input_value: dict[str, Json]) -> Json:
    key = _text(input_value.get("key"), "key")
    collection = _text(input_value.get("collection"), "collection")
    threshold = _number(input_value.get("threshold"), "threshold")
    api_base = _text(input_value.get("api_base", DEFAULT_API_BASE), "api_base")
    numbers = cast(list[float], _array(input_value.get("vector"), "vector"))
    vector = [float(item) for item in numbers]
    messages = _array(input_value.get("messages"), "messages")
    raw_metadata = input_value.get("metadata")
    metadata = None if raw_metadata is None else _object(raw_metadata, "metadata")
    client = _FakeClient(_responses(input_value.get("responses")))
    kwargs: dict[str, Json] = {"messages": messages}
    if metadata is not None:
        kwargs["metadata"] = metadata
    with (
        patch(
            "litellm.llms.custom_httpx.http_handler._get_httpx_client",
            _client_factory(client),
        ),
        patch(
            "litellm.llms.custom_httpx.http_handler.get_async_httpx_client",
            _client_factory(client),
        ),
        patch("litellm.embedding", _embedding(vector)),
    ):
        cache = QdrantSemanticCache(
            collection_name=collection,
            similarity_threshold=threshold,
            qdrant_api_base=api_base,
        )
        value = cast(Json, cast(_CacheOps, cache).get_cache(key, **kwargs))
    search: Json = None
    call = _search_call(client)
    if call is not None:
        search = {"url": call["url"], "body": call["json"]}
    return {
        "value": value,
        "similarity": None if metadata is None else metadata.get("semantic-similarity"),
        "search": search,
    }
