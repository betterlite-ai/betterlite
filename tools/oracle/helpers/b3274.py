from typing import TypedDict

import litellm
from litellm.search import search_provider_cost_per_query
from pydantic import JsonValue


class Input(TypedDict):
    model_cost: dict[str, dict[str, JsonValue]]
    model: str
    custom_llm_provider: str | None
    number_of_queries: int
    optional_params: dict[str, JsonValue]


def run(data: Input) -> JsonValue:
    litellm.model_cost = data["model_cost"]
    input_cost, output_cost = search_provider_cost_per_query(
        model=data["model"],
        custom_llm_provider=data["custom_llm_provider"],
        number_of_queries=data["number_of_queries"],
        optional_params=data["optional_params"],
    )
    return {"input_cost": round(input_cost, 9), "output_cost": round(output_cost, 9)}
