import asyncio
from collections.abc import Mapping
from typing import Literal, Protocol

import litellm
from litellm.proxy.spend_tracking import budget_reservation
from litellm.router import Router
from pydantic import Field, JsonValue
from typing_extensions import override

from probes.protocol import Operation, Strict

litellm.suppress_debug_info = True


class BudgetEstimates(Protocol):
    def estimate_request_max_cost(
        self,
        request_body: dict[str, JsonValue],
        route: str,
        llm_router: Router | None,
        input_token_counts: Mapping[str, int] | None = None,
    ) -> float | None: ...

    def estimate_request_input_cost(
        self,
        request_body: dict[str, JsonValue],
        route: str,
        llm_router: Router | None,
        input_token_counts: Mapping[str, int] | None = None,
    ) -> float | None: ...

    async def count_request_input_tokens(
        self,
        request_body: dict[str, JsonValue],
        route: str,
        llm_router: Router | None,
        raw_body: bytes | None = None,
    ) -> Mapping[str, int]: ...


class Deployment(Strict):
    model_name: str
    backend_model: str
    model_info: dict[str, JsonValue]


def deployment_entry(deployment: Deployment) -> dict[str, JsonValue]:
    return {
        "model_name": deployment.model_name,
        "litellm_params": {"model": deployment.backend_model},
        "model_info": deployment.model_info,
    }


class CostOutput(Strict):
    cost: float | None


class CountOutput(Strict):
    counts: dict[str, int]


class ReservationCost(Operation):
    function: Literal["max_cost", "input_cost", "count"]
    request: dict[str, JsonValue]
    route: str
    model_cost: dict[str, dict[str, JsonValue]]
    input_token_counts: dict[str, int] | None = None
    model_list: list[Deployment] = Field(default_factory=list)

    @override
    def run(self) -> Strict:
        litellm.model_cost = dict(self.model_cost)
        return estimate(budget_reservation, self)

    def build_router(self) -> Router | None:
        if not self.model_list:
            return None
        entries: list[dict[str, JsonValue]] = [
            deployment_entry(deployment) for deployment in self.model_list
        ]
        return Router(model_list=entries)


def estimate(budget: BudgetEstimates, operation: ReservationCost) -> Strict:
    router = operation.build_router()
    if operation.function == "max_cost":
        return CostOutput(
            cost=budget.estimate_request_max_cost(
                request_body=operation.request,
                route=operation.route,
                llm_router=router,
                input_token_counts=operation.input_token_counts,
            )
        )
    if operation.function == "input_cost":
        return CostOutput(
            cost=budget.estimate_request_input_cost(
                request_body=operation.request,
                route=operation.route,
                llm_router=router,
                input_token_counts=operation.input_token_counts,
            )
        )
    counts = asyncio.run(
        budget.count_request_input_tokens(
            request_body=operation.request,
            route=operation.route,
            llm_router=router,
        )
    )
    return CountOutput(counts=dict(counts))
