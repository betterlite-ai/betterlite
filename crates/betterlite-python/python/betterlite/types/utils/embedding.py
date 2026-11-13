from collections.abc import Mapping, Sequence
from typing import Literal

from pydantic import Field, JsonValue, PrivateAttr, SerializeAsAny
from typing_extensions import override

from betterlite.types.utils.base import (
    DictJsonObject,
    Dumpable,
    InitValue,
    ModelValue,
    ResponseObject,
    dump_value,
)
from betterlite.types.utils.usage import Usage


class Embedding(ResponseObject):
    embedding: list[JsonValue] | str = Field(default_factory=list)
    index: int
    object: Literal["embedding"]


class EmbeddingResponse(DictJsonObject):
    model: str | None = None
    # LiteLLM keeps the Embedding models the caller sends, so a reader can still reach their fields
    data: list[JsonValue | SerializeAsAny[ResponseObject]]
    object: Literal["list"]
    usage: Usage | None = None
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default_factory=dict)
    _response_headers: dict[str, JsonValue] | None = PrivateAttr(default=None)
    _response_ms: float | None = PrivateAttr(default=None)

    @override
    def __init__(
        self, model: JsonValue = None, usage: ModelValue = None,
        response_ms: float | None = None, data: Sequence[ModelValue] | None = None,
        hidden_params: Mapping[str, JsonValue] | None = None,
        _response_headers: Mapping[str, JsonValue] | None = None,
        **_params: InitValue,
    ) -> None:
        super().__init__(
            model=model, object="list", data=embedding_items(data),
            usage=Usage() if usage is None else dump_value(usage),
        )
        if hidden_params is not None:
            self._hidden_params = dict(hidden_params)
        if _response_headers is not None:
            self._response_headers = dict(_response_headers)
        self._response_ms = response_ms


def embedding_items(
    data: Sequence[ModelValue] | None,
) -> list[JsonValue | ResponseObject]:
    if data is None:
        return []
    items: list[JsonValue | ResponseObject] = []
    for item in data:
        match item:
            case ResponseObject():
                items.append(item)
            case Dumpable():
                items.append(item.model_dump())
            case dict() | list() | bool() | int() | float() | str() | None:
                items.append(item)
    return items
