import litellm
from pydantic import JsonValue as Json

from .contracts import Input
from .scenario import scenario

litellm.suppress_debug_info = True


def run(input_value: Input) -> Json:
    scenarios = input_value["scenarios"]
    return [scenario(entry) for entry in scenarios]
