from typing import ClassVar

from not_built import Unbuilt


class CompactTransportRequest(Unbuilt):
    task: ClassVar[str] = "B2418"

    responses_compact_transport: None
