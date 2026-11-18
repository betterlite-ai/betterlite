from typing import ClassVar

from not_built import Unbuilt


class VectorRetrievalContextRequest(Unbuilt):
    task: ClassVar[str] = "B1077"

    vector_retrieval_context: None
