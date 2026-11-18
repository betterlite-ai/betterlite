from typing import ClassVar

from not_built import Unbuilt


class RerankHttpRequest(Unbuilt):
    task: ClassVar[str] = "B2454"

    rerank_http: None
