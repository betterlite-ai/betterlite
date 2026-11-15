from litellm.llms.databricks import cost_calculator
from litellm.types.utils import Usage
from pydantic import JsonValue, TypeAdapter

from .common import CostCase, Input, cases_of

USAGE = Usage(prompt_tokens=1_000, completion_tokens=1_000, total_tokens=2_000)


def format_cost(value: float) -> str:
    rendered = f"{value:.12f}"
    return rendered.rstrip("0").rstrip(".")


def cost(input_value: Input) -> JsonValue:
    cases = TypeAdapter(list[CostCase]).validate_python(cases_of(input_value))
    results: list[JsonValue] = []
    for case in cases:
        tier = case.get("service_tier")
        priced: list[JsonValue] = []
        for model in case["models"]:
            prompt, completion = cost_calculator.cost_per_token(
                model=model, usage=USAGE, service_tier=tier
            )
            entry: dict[str, JsonValue] = {
                "prompt": format_cost(prompt),
                "completion": format_cost(completion),
            }
            priced.append(entry)
        results.append(priced)
    return results
