import json
from collections.abc import Mapping
from dataclasses import dataclass
from typing import Final

import betterlite
from betterlite._completion.errors import UnsupportedCallArgument
from betterlite._endpoints import CallValue
from betterlite._endpoints.payload import PathValue, Readable, is_file_tuple, is_json_container

MAX_REQUEST_BYTES: Final = 8 * 1024 * 1024
MAX_HEADERS: Final = 256
MAX_DEPTH: Final = 64
STRING_CHUNK: Final = 1024


@dataclass
class RequestBudget:
    remaining: int = MAX_REQUEST_BYTES

    def charge(self, size: int) -> None:
        self.remaining -= size
        if self.remaining < 0:
            raise ValueError(f"rerank request exceeds {MAX_REQUEST_BYTES} bytes")

    def consume(self, value: CallValue, depth: int = 0, field: str = "rerank") -> None:
        if depth > MAX_DEPTH:
            raise ValueError(f"rerank request exceeds {MAX_DEPTH} nesting levels")
        if is_json_container(value):
            match value:
                case dict():
                    self.consume_fields(value, depth)
                case list():
                    self.charge(2 + max(0, len(value) - 1) * 2)
                    for item in value:
                        self.consume(item, depth + 1, field)
            return
        self.consume_scalar(value, field)

    def consume_scalar(self, value: CallValue, field: str) -> None:
        if is_file_tuple(value):
            raise UnsupportedCallArgument(field)
        match value:
            case str():
                # Count raw text first so escaping only allocates bounded chunks.
                self.charge(len(value) + 2)
                for offset in range(0, len(value), STRING_CHUNK):
                    chunk = value[offset:offset + STRING_CHUNK]
                    self.charge(len(json.dumps(chunk)) - len(chunk) - 2)
            case bool() | float() | None:
                self.charge(len(json.dumps(value)))
            case int():
                if value.bit_length() > self.remaining * 4:
                    raise ValueError(f"rerank request exceeds {MAX_REQUEST_BYTES} bytes")
                self.charge(len(json.dumps(value)))
            case bytes() | bytearray() | PathValue() | Readable():
                raise UnsupportedCallArgument(field)
            case object():
                return

    def consume_fields(self, fields: Mapping[str, CallValue], depth: int = 0) -> None:
        self.charge(2 + max(0, len(fields) - 1) * 2 + len(fields) * 2)
        for name, value in fields.items():
            self.consume(name, depth + 1)
            self.consume(value, depth + 1, name)


def rerank_settings() -> dict[str, CallValue]:
    # Only read the settings this endpoint uses; read_settings validates unrelated globals too.
    return {
        "api_base": betterlite.api_base,
        "api_key": betterlite.api_key,
        "cohere_key": betterlite.cohere_key,
        "drop_params": betterlite.drop_params,
        "headers": betterlite.headers,
        "infinity_key": betterlite.infinity_key,
        "togetherai_api_key": betterlite.togetherai_api_key,
    }


def check_request_size(values: tuple[CallValue, ...], kwargs: Mapping[str, CallValue]) -> None:
    headers = 0
    for source in (kwargs.get("headers"), betterlite.headers):
        match source:
            case Mapping():
                headers += len(source)
            case object():
                continue
    if headers > MAX_HEADERS:
        raise ValueError(f"too many rerank headers: {headers} > {MAX_HEADERS}")
    budget = RequestBudget()
    budget.consume_fields(kwargs)
    budget.consume_fields(rerank_settings())
    for value in values:
        budget.consume(value)
