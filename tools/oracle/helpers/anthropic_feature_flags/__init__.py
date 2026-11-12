import sys
from contextlib import redirect_stdout

import litellm
from pydantic import JsonValue as Json

from .answers import answer
from .contracts import Input


def run(input_value: Input) -> Json:
    previous = litellm.model_cost  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L563
    litellm.model_cost = input_value["fixture"]["model_cost"]
    try:
        # LiteLLM prints the provider list for an unroutable model; keep stdout for the JSON answer
        with redirect_stdout(sys.stderr):
            return {"calls": [answer(call) for call in input_value["calls"]]}
    finally:
        litellm.model_cost = previous
