from pydantic import JsonValue

from azure_assistants.calls import Input
from azure_assistants.steps import steps_of, steps_of_variant


def run(argument: Input) -> JsonValue:
    steps = steps_of_variant(argument)
    return steps_of(steps)
