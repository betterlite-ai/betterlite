import asyncio

import litellm
from pydantic import JsonValue

from .models import Input, JsonAdapter
from .scenario import run_all

litellm.suppress_debug_info = True


def run(input_value: Input) -> JsonValue:
    results = asyncio.run(run_all(input_value))
    return JsonAdapter.validate_python({"runs": results})
