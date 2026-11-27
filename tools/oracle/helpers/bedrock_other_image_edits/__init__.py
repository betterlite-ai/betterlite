import litellm
from pydantic import JsonValue

from .answers import ANSWERS
from .contracts import PROBE, Input

# assigning model_cost registers every provider and prints the list
litellm.suppress_debug_info = True


def run(input_value: Input) -> JsonValue:
    litellm.model_cost = input_value["model_cost"]
    return [
        ANSWERS[input_value["function"]](PROBE.validate_python(probe))
        for probe in input_value["probes"]
    ]
