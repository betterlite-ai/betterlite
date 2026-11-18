from typing import ClassVar

from not_built import Unbuilt


class VectorStoreReadRequest(Unbuilt):
    task: ClassVar[str] = "B1065"

    vector_store_read: None
