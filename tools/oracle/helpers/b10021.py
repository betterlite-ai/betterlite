from typing import NotRequired, TypedDict

from litellm.proxy.management_helpers.auto_router_availability import build_auto_router_catalog
from pydantic import JsonValue, TypeAdapter


class Row(TypedDict):
    model_id: str
    model_name: str
    litellm_params: dict[str, JsonValue]
    model_info: NotRequired[dict[str, JsonValue] | None]
    created_by: NotRequired[str | None]


class Input(TypedDict):
    rows: list[Row]


class Deployment(TypedDict):
    litellm_params: dict[str, JsonValue]
    model_info: dict[str, JsonValue]


class Answer(TypedDict):
    model_id: str
    team_id: str | None
    created_by: str | None
    deployment: Deployment


def run(input_value: Input) -> list[Answer] | None:
    catalog = build_auto_router_catalog(input_value["rows"])
    if catalog is None:
        return None
    return [
        Answer(
            model_id=entry.model_id,
            team_id=entry.team_id,
            created_by=entry.created_by,
            deployment=TypeAdapter(Deployment).validate_python(entry.deployment),
        )
        for entry in catalog
    ]
