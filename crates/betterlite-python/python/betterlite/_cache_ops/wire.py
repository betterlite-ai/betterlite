import json
from collections.abc import Mapping, Sequence
from typing import Final

from pydantic import JsonValue, TypeAdapter

from betterlite._cache_ops.types import StoredAnswer
from betterlite.types.utils.base import Dumpable, InitValue, dump_value

_STORED: Final[TypeAdapter[StoredAnswer]] = TypeAdapter(StoredAnswer)
_VALUE: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
_SEMANTIC: Final[TypeAdapter[tuple[JsonValue, float | None]]] = TypeAdapter(
    tuple[JsonValue, float | None]
)
_LOOKUP_PARAMS: Final[tuple[str, ...]] = ("messages", "input")
_SIMILARITY: Final = "semantic-similarity"


# A pydantic response object keeps its model_dump(); the rest of a request keeps its JSON values.
def dumped(value: InitValue) -> JsonValue:
    answer = dump_value(value)
    match answer:
        case None | str() | int() | float():
            return answer
        case Mapping() as fields:
            return {name: dumped(item) for name, item in fields.items()}
        case Sequence() as items:
            return [dumped(item) for item in items]
        case Dumpable():
            raise TypeError("dump_value answers JSON for a model")


def text_of(body: dict[str, JsonValue]) -> str:  # ast-grep-ignore: no-one-line-functions
    return json.dumps(body, default=str)


# ast-grep-ignore: no-one-line-functions - key derivation skips a lookup's cache_key
def key_arguments(kwargs: Mapping[str, JsonValue]) -> dict[str, JsonValue]:
    return {name: value for name, value in kwargs.items() if name != "cache_key"}


# ast-grep-ignore: no-one-line-functions
def write_call(kwargs: Mapping[str, JsonValue], result: InitValue) -> str:
    return text_of({"kwargs": dict(kwargs), "result": dumped(result)})


def stored_of(text: str) -> StoredAnswer:  # ast-grep-ignore: no-one-line-functions
    return _STORED.validate_json(text)


def value_of(text: str) -> JsonValue:  # ast-grep-ignore: no-one-line-functions
    return _VALUE.validate_json(text)


# ast-grep-ignore: no-one-line-functions
def semantic_answer(text: str) -> tuple[JsonValue, float | None]:
    return _SEMANTIC.validate_json(text)


def stored_arguments(
    kwargs: Mapping[str, JsonValue], answer: StoredAnswer
) -> dict[str, JsonValue]:
    stored = dict(kwargs)
    ttl = answer["ttl"]
    if ttl is not None:
        stored["ttl"] = ttl
    return stored


# A backend gets only the prompt and a copy of the metadata, so it cannot change the request.
def lookup_arguments(kwargs: Mapping[str, JsonValue]) -> dict[str, JsonValue]:
    lookup: dict[str, JsonValue] = {}
    for name in _LOOKUP_PARAMS:
        if name in kwargs:
            lookup[name] = kwargs[name]
    match kwargs.get("metadata"):
        case Mapping() as metadata:
            lookup["metadata"] = dict(metadata)
        case None | str() | int() | float() | list():
            return lookup
    return lookup


# A similarity a backend wrote into the metadata copy goes back onto the request's metadata.
def carry_similarity(original: dict[str, JsonValue], lookup: dict[str, JsonValue]) -> None:
    answered = _object_fields(lookup.get("metadata"))
    if _SIMILARITY in answered:
        _object_fields(original.get("metadata"))[_SIMILARITY] = answered[_SIMILARITY]


# LiteLLM's _stamp_semantic_similarity writes a native lookup's similarity onto the metadata.
def stamp_similarity(kwargs: dict[str, JsonValue], similarity: float | None) -> None:
    if similarity is not None:
        _object_fields(kwargs.get("metadata"))[_SIMILARITY] = similarity


# A metadata that is absent or not an object takes the write without keeping it.
def _object_fields(value: JsonValue | None) -> dict[str, JsonValue]:
    match value:
        case dict() as fields:
            return fields
        case None | str() | int() | float() | list():
            return {}


# LiteLLM counts a list input by its items and any other input as one embedding request.
def embedding_inputs(kwargs: Mapping[str, JsonValue]) -> tuple[int, list[JsonValue]]:
    match kwargs["input"]:
        case list() as items:
            return len(items), [*items]
        case str() as text:
            return 1, [text]
        case None | bool() | int() | float() | dict():
            return 1, []


def embedding_records(result: InitValue) -> list[JsonValue]:
    match dumped(result):
        case {"data": list() as records}:
            return [*records]
        case dict() | list() | str() | bool() | int() | float() | None:
            return []
