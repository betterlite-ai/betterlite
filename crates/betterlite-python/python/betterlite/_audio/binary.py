from collections.abc import Iterator
from os import PathLike
from pathlib import Path
from typing import Final

from pydantic import JsonValue, TypeAdapter

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
DEFAULT_CHUNK_SIZE: Final = 1024 * 1024


# LiteLLM hands back openai's HttpxBinaryResponseContent: the provider's bytes with their status and headers.
class HttpxBinaryResponseContent:
    def __init__(self, content: bytes, status_code: int, headers: dict[str, str]) -> None:
        self._content = content
        self.status_code = status_code
        self.headers = headers

    @property
    def content(self) -> bytes:
        return self._content

    @property
    def text(self) -> str:
        return self._content.decode("utf-8", "replace")

    # ast-grep-ignore: no-one-line-functions
    def read(self) -> bytes:
        return self._content

    # ast-grep-ignore: no-one-line-functions
    def json(self) -> JsonValue:
        return JSON.validate_json(self._content)

    def iter_bytes(self, chunk_size: int | None = None) -> Iterator[bytes]:
        size = DEFAULT_CHUNK_SIZE if chunk_size is None else chunk_size
        for start in range(0, len(self._content), size):
            yield self._content[start:start + size]

    def write_to_file(self, file: str | PathLike[str]) -> None:
        with Path(file).open("wb") as handle:
            _ = handle.write(self._content)

    def stream_to_file(self, file: str | PathLike[str], chunk_size: int | None = None) -> None:
        with Path(file).open("wb") as handle:
            for chunk in self.iter_bytes(chunk_size):
                _ = handle.write(chunk)
