import json
from typing import cast

import litellm
from litellm.utils import _get_model_info_helper  # pyright: ignore[reportPrivateUsage]

Json = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    key = cast(str, input_value["key"])
    model = cast(str, input_value["model"])
    provider = cast(str, input_value["provider"])
    entry = cast(dict[str, Json], input_value["entry"])
    costs = cast(dict[str, dict[str, Json]], litellm.model_cost)
    original = dict(costs)
    try:
        costs.clear()
        costs[key] = entry
        result = _get_model_info_helper(model=model, custom_llm_provider=provider)
        return cast(Json, json.loads(json.dumps(result)))
    finally:
        costs.clear()
        costs.update(original)
