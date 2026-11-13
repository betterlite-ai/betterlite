from collections.abc import Mapping
from typing import ClassVar, Literal

from typing_extensions import override

from betterlite.types.utils.base import ResponseObject
from betterlite.types.utils.usage import Usage
from betterlite.types.utils.values import JsonValue, StoredValue


class Embedding(ResponseObject):
    embedding: list[JsonValue] | str
    index: int
    object: Literal["embedding"]
    defaults: ClassVar[dict[str, StoredValue]] = {"embedding": []}


class EmbeddingResponse(ResponseObject):
    model: str | None
    data: list[JsonValue]
    object: Literal["list"]
    usage: Usage | None
    _hidden_params: dict[str, StoredValue]
    _response_headers: dict[str, StoredValue] | None
    _response_ms: float | None
    defaults: ClassVar[dict[str, StoredValue]] = {
        "_hidden_params": {}, "_response_headers": None, "_response_ms": None,
    }

    @override
    def __init__(
        self, model: StoredValue = None, usage: StoredValue = None,
        response_ms: float | None = None, data: StoredValue = None,
        hidden_params: Mapping[str, StoredValue] | None = None,
        _response_headers: Mapping[str, StoredValue] | None = None,
        **_params: StoredValue,
    ) -> None:
        super().__init__(
            model=model,
            object="list",
            data=[] if data is None else data,
            usage=Usage() if usage is None else usage,
        )
        if hidden_params is not None:
            self._hidden_params = dict(hidden_params)
        if _response_headers is not None:
            self._response_headers = dict(_response_headers)
        self._response_ms = response_ms
