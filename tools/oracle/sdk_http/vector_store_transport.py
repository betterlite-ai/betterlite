from typing import ClassVar

from not_built import Unbuilt


class VectorStoreTransportRequest(Unbuilt):
    task: ClassVar[str] = "B2402"

    vector_store_transport: None
