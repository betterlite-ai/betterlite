from typing import Final, Literal, NotRequired, TypedDict

from pydantic import JsonValue, TypeAdapter

OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])

Function = Literal[
    "key_fallback",
    "user_budget",
    "team_budget",
    "end_user_apply",
    "end_user_update",
    "global_spend",
]


class Call(TypedDict):
    function: Function
    token: NotRequired[dict[str, JsonValue] | None]
    models: NotRequired[list[str] | None]
    model_name: NotRequired[str | None]
    request_model: NotRequired[str | None]
    request_data: NotRequired[dict[str, JsonValue] | None]
    path_params: NotRequired[dict[str, JsonValue] | None]
    failing: NotRequired[bool | None]
    fallback: NotRequired[str | None]
    failing_models: NotRequired[list[str] | None]
    budget: NotRequired[dict[str, JsonValue] | None]
    params: NotRequired[dict[str, JsonValue] | None]
    end_user_id: NotRequired[str | None]
    max_budget: NotRequired[float | None]
    spend: NotRequired[float | None]
    database: NotRequired[bool | None]
    admin_name: NotRequired[str | None]
    tokens: NotRequired[list[str] | None]
    aliases: NotRequired[dict[str, str] | None]
    max_recurse_depth: NotRequired[int | None]


class Request(TypedDict):
    calls: list[Call]
