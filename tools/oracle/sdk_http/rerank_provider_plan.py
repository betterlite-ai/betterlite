from typing import ClassVar

from not_built import Unbuilt


class RerankProviderPlanRequest(Unbuilt):
    task: ClassVar[str] = "B1079"

    rerank_provider_plan: None
