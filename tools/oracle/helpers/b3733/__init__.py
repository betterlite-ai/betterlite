import asyncio
from typing import TypeAlias

from pydantic import JsonValue as Json

from .plan import PlanCall, plan
from .sanitize import SanitizeCall, sanitize

Input: TypeAlias = PlanCall | SanitizeCall


def run(input_value: Input) -> Json:
    answer: Json = None
    match input_value["function"]:
        case "plan_file_scans":
            answer = plan(input_value)
        case "sanitize_request":
            answer = asyncio.run(sanitize(input_value))
    return answer
