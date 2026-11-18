from typing import ClassVar

from not_built import Unbuilt


class VectorStoreSearchCreateRequest(Unbuilt):
    task: ClassVar[str] = "B1064"

    vector_store_search_create: None
