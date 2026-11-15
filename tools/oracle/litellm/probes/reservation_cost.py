import asyncio
from typing import Literal

import litellm
from litellm.proxy.spend_tracking.budget_reservation import (
    count_request_input_tokens,
    estimate_request_input_cost,
    estimate_request_max_cost,
)
from litellm.router import Router
from pydantic import Field, JsonValue
from typing_extensions import override

from probes.protocol import Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

litellm.suppress_debug_info = True


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
        router = self.build_router()
        if self.function == "max_cost":
            return CostOutput(
                cost=estimate_request_max_cost(
                    request_body=self.request,
                    route=self.route,
                    llm_router=router,
                    input_token_counts=self.input_token_counts,
                )
            )
        if self.function == "input_cost":
            return CostOutput(
                cost=estimate_request_input_cost(
                    request_body=self.request,
                    route=self.route,
                    llm_router=router,
                    input_token_counts=self.input_token_counts,
                )
            )
        counts = asyncio.run(
            count_request_input_tokens(
                request_body=self.request,
                route=self.route,
                llm_router=router,
            )
        )
        return CountOutput(counts=dict(counts))

    def build_router(self) -> Router | None:
        if not self.model_list:
            return None
        entries: list[dict[str, JsonValue]] = [
            deployment_entry(deployment) for deployment in self.model_list
        ]
        return Router(model_list=entries)
