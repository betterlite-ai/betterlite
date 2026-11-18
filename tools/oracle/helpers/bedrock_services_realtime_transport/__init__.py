import asyncio

import litellm
from pydantic import JsonValue

from .models import Input
from .scenario import run_all

litellm.suppress_debug_info = True


def run(input_value: Input) -> JsonValue:
    results: list[JsonValue] = asyncio.run(run_all(input_value))
    return {"runs": results}
