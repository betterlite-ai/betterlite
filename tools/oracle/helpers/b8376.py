from typing import TypedDict

from litellm.proxy import proxy_server
from pydantic import JsonValue


class Input(TypedDict, total=False):
    user_id: str
    end_user_id: str
    team_id: str
    tags: list[JsonValue]
    response_cost: float


def run(input_value: Input) -> tuple[str, ...]:
    return proxy_server.update_cache_read_keys(
        user_id=input_value.get("user_id"),
        end_user_id=input_value.get("end_user_id"),
        team_id=input_value.get("team_id"),
        tags=input_value.get("tags"),
        response_cost=input_value.get("response_cost"),
    )
