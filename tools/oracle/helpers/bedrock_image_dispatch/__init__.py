from typing import Final

import litellm
from pydantic import JsonValue

from .models import PROBE, Input
from .rows import case_row

GENERATION_CASES: Final = ("titan_image", "stability_image", "canvas_image")


def run(input_value: Input) -> JsonValue:
    litellm.model_cost = input_value["model_cost"]
    edit = input_value["case"] not in GENERATION_CASES
    rows: list[JsonValue] = []
    for probe in input_value["probes"]:
        parsed = PROBE.validate_python(probe)
        rows.append(case_row(parsed, edit=edit))
    return rows
