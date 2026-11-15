import asyncio
from typing import Literal, TypedDict

import litellm
from litellm.proxy.spend_tracking.budget_reservation import (
    count_request_input_tokens,
    estimate_request_input_cost,
    estimate_request_max_cost,
)
from litellm.router import Router
from pydantic import JsonValue

litellm.suppress_debug_info = True


class Deployment(TypedDict):
    model_name: str
    backend_model: str
    model_info: dict[str, JsonValue]


class Input(TypedDict):
    function: Literal["max_cost", "input_cost", "count"]
    request: dict[str, JsonValue]
    route: str
    model_cost: dict[str, dict[str, JsonValue]]
    input_token_counts: dict[str, int] | None
    model_list: list[Deployment]


def run(input_value: Input) -> float | None | dict[str, int]:
    install_model_cost(input_value["model_cost"])
    router = build_router(input_value["model_list"])
    function = input_value["function"]
    if function == "max_cost":
        return rounded(
            estimate_request_max_cost(
                request_body=input_value["request"],
                route=input_value["route"],
                llm_router=router,
                input_token_counts=input_value["input_token_counts"],
            )
        )
    if function == "input_cost":
        return rounded(
            estimate_request_input_cost(
                request_body=input_value["request"],
                route=input_value["route"],
                llm_router=router,
                input_token_counts=input_value["input_token_counts"],
            )
        )
    counts = asyncio.run(
        count_request_input_tokens(
            request_body=input_value["request"],
            route=input_value["route"],
            llm_router=router,
        )
    )
    return dict(counts)


def rounded(cost: float | None) -> float | None:
    if cost is None:
        return None
    return round(cost, 9)


def install_model_cost(entries: dict[str, dict[str, JsonValue]]) -> None:
    litellm.model_cost = dict(entries)


def build_router(deployments: list[Deployment]) -> Router | None:
    if not deployments:
        return None
    entries: list[dict[str, JsonValue]] = []
    for deployment in deployments:
        entry: dict[str, JsonValue] = {}
        entry["model_name"] = deployment["model_name"]
        entry["litellm_params"] = {"model": deployment["backend_model"]}
        entry["model_info"] = deployment["model_info"]
        entries.append(entry)
    return Router(model_list=entries)
