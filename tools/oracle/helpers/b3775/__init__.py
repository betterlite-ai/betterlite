from typing import TypeAlias

from pydantic import JsonValue

from b3775 import headroom, hiddenlayer
from b3775.inputs import HiddenlayerInput, PlanInput, ShouldRunInput

Input: TypeAlias = ShouldRunInput | PlanInput | HiddenlayerInput


def run(input_value: Input) -> JsonValue:
    match input_value["kind"]:
        case "should_run":
            return headroom.should_run(input_value)
        case "plan":
            return headroom.plan(input_value)
        case "hiddenlayer":
            return hiddenlayer.run(input_value)
