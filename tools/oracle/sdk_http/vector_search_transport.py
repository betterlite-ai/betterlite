from typing import ClassVar

from not_built import Unbuilt


class VectorSearchTransportRequest(Unbuilt):
    task: ClassVar[str] = "B2484"

    vector_search_transport: None
