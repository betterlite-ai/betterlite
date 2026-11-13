from collections.abc import Callable, Mapping
from types import ModuleType
from typing import Protocol, TypeAlias, cast

import litellm
from litellm.llms.cohere.common_utils import (
    CohereV2ModelResponseIterator,
    ModelResponseIterator,
    validate_environment,  # pyright: ignore[reportUnknownVariableType]
)
from litellm.types.utils import GenericStreamingChunk

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class CohereIterator(Protocol):
    def convert_str_chunk_to_generic_chunk(
        self, chunk: str | bytes
    ) -> GenericStreamingChunk: ...


def run(call: dict[str, Json]) -> Json:
    _litellm_module = cast(ModuleType, litellm)
    version = call.get("version", "v1")
    record = call.get("record", "")
    if not isinstance(version, str) or not isinstance(record, str):
        raise ValueError("version and record must be strings")
    if version == "v1":
        iterator = cast(
            CohereIterator,
            ModelResponseIterator([record], sync_stream=True),
        )
    elif version == "v2":
        iterator = cast(
            CohereIterator,
            CohereV2ModelResponseIterator([record], sync_stream=True),
        )
    else:
        raise ValueError("version must be v1 or v2")
    raw_record: str | bytes = record.encode("utf-8") if call.get("bytes") is True else record
    converted = iterator.convert_str_chunk_to_generic_chunk(raw_record)
    headers_value = call.get("headers", {})
    headers: dict[str, object] = {}
    if isinstance(headers_value, Mapping):
        headers = {str(key): value for key, value in headers_value.items()}
    api_key = call.get("api_key", "")
    if not isinstance(api_key, str):
        raise ValueError("api_key must be a string")
    validate = cast(
        Callable[[dict[str, object], str, list[object], dict[str, object], str], dict[str, object]],
        validate_environment,
    )
    validated = validate(headers, "model", [], {}, api_key)
    return {"headers": cast(Json, validated), "chunk": cast(Json, converted)}
