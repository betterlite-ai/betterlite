from typing import Literal, TypedDict

from typing_extensions import ReadOnly


class BinaryResponseSummary(TypedDict):
    object: ReadOnly[Literal["binary"]]
    content_type: ReadOnly[str | None]
    num_bytes: ReadOnly[int]
