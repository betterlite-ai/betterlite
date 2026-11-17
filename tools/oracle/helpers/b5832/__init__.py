import litellm
from pydantic import JsonValue

from b5832.answers import step_value
from b5832.records import INPUT, Input, model_cost_of


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    parsed: Input = INPUT.validate_python(input_value)
    litellm.model_cost = model_cost_of(parsed)
    return [step_value(step) for step in parsed["steps"]]
