from typing import ClassVar

from not_built import Unbuilt


class VectorStoreWriteRequest(Unbuilt):
    task: ClassVar[str] = "B1066"

    vector_store_write: None
