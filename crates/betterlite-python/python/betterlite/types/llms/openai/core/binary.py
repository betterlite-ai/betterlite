from typing import Literal

from typing_extensions import ReadOnly, TypedDict


class BinaryResponseSummary(TypedDict):
    object: ReadOnly[Literal["binary"]]
    content_type: ReadOnly[str | None]
    num_bytes: ReadOnly[int]
