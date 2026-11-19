import litellm
from pydantic import JsonValue

from .contracts import Input
from .steps import run_step

litellm.suppress_debug_info = True


def run(input_value: Input) -> JsonValue:
    steps: list[JsonValue] = [run_step(step) for step in input_value["steps"]]
    return {"steps": steps}
