import asyncio
from typing import Final, TypedDict

from litellm.proxy import proxy_server
from litellm.proxy._types import CostEstimateRequest, UserAPIKeyAuth
from litellm.proxy.management_endpoints import cost_tracking_settings
from litellm.router import Router
from pydantic import JsonValue, TypeAdapter

JSON: Final = TypeAdapter(JsonValue)


class Input(TypedDict):
    model_list: list[dict[str, JsonValue]]
    request: dict[str, JsonValue]


def run(input_value: Input) -> JsonValue:
    proxy_server.llm_router = Router(model_list=input_value["model_list"])
    request = CostEstimateRequest.model_validate(input_value["request"])
    response = asyncio.run(
        cost_tracking_settings.estimate_cost(
            request=request, user_api_key_dict=UserAPIKeyAuth()
        )
    )
    return JSON.validate_json(response.model_dump_json())
