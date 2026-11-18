from typing import ClassVar

from not_built import Unbuilt


class RerankCallRequest(Unbuilt):
    task: ClassVar[str] = "B1014"

    rerank_call: None
