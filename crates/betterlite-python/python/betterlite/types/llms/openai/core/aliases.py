from collections.abc import Iterable, Mapping
from os import PathLike
from typing import IO, TypeAlias

FileContent: TypeAlias = IO[bytes] | bytes | PathLike[str] | PathLike[bytes]

FileTypes: TypeAlias = (
    FileContent
    | tuple[str | None, FileContent]
    | tuple[str | None, FileContent, str | None]
    | tuple[str | None, FileContent, str | None, Mapping[str, str]]
)

EmbeddingInput: TypeAlias = str | list[str]

AllPromptValues: TypeAlias = str | list[str] | Iterable[int] | Iterable[Iterable[int]] | None
