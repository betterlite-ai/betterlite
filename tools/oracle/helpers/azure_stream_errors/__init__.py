import asyncio

import litellm
from pydantic import JsonValue

from .handler import run_cases
from .types import Input

litellm.suppress_debug_info = True


def run(input_value: Input) -> JsonValue:
    return asyncio.run(run_cases(input_value))
