import json
from typing import Final, Literal, TypedDict

import litellm
from litellm.router import Router
from litellm.types.router import Deployment, DeploymentTypedDict, RouterModelGroupAliasItem
from pydantic import JsonValue, TypeAdapter

LOOKUP_FUNCTIONS: Final = frozenset(
    {
        "get_deployment",
        "get_deployment_by_model_group_name",
        "get_model_ids",
        "has_model_id",
        "resolve_model_name_from_model_id",
    }
)
STALE_ID: Final = "ghost-id"
STALE_INDEX: Final = 99
STALE_NAME: Final = "ghost"


class Arguments(TypedDict, total=False):
    model_id: str
    model_name: str | None
    team_id: str | None
    exclude_team_models: bool
    candidate_id: str
    deployment_index: int


class Input(TypedDict):
    function: Literal[
        "get_deployment",
        "get_deployment_by_model_group_name",
        "get_model_ids",
        "get_candidate_model_ids_for_route",
        "has_model_id",
        "resolve_model_name_from_model_id",
        "get_model_names",
        "get_model_list",
        "get_model_list_from_model_alias",
        "should_include_deployment",
    ]
    model_list: list[dict[str, JsonValue]]
    model_group_alias: dict[str, str | RouterModelGroupAliasItem]
    routing_groups: list[dict[str, JsonValue]]
    inferred_provider: str | None
    stale_index: bool
    arguments: Arguments


class ParamsView(TypedDict):
    model: str


class InfoView(TypedDict, total=False):
    id: str | None
    team_id: str | None
    team_public_model_name: str | None


class DeploymentView(TypedDict):
    model_name: str
    litellm_params: ParamsView
    model_info: InfoView


VIEW: Final = TypeAdapter(DeploymentView)


def required(value: str | None, name: str) -> str:
    if value is None:
        raise ValueError(f"arguments.{name} is required")
    return value


def row_view(value: DeploymentView) -> dict[str, JsonValue]:
    info = value["model_info"]
    row: dict[str, JsonValue] = {
        "model_name": value["model_name"],
        "model": value["litellm_params"]["model"],
        "id": info.get("id"),
        "team_id": info.get("team_id"),
        "public_name": info.get("team_public_model_name"),
    }
    return row


def optional_row(value: Deployment | None) -> dict[str, JsonValue] | None:
    if value is None:
        return None
    return row_view(VIEW.validate_json(value.model_dump_json()))


def listing_rows(rows: list[DeploymentTypedDict] | None) -> list[JsonValue]:
    projected: list[JsonValue] = []
    if rows is None:
        return projected
    projected.extend(row_view(VIEW.validate_json(json.dumps(row))) for row in rows)
    return projected


def lookup_answer(router: Router, call: Input) -> JsonValue:
    arguments = call["arguments"]
    match call["function"]:
        case "get_deployment":
            return optional_row(router.get_deployment(required(arguments.get("model_id"), "model_id")))
        case "get_deployment_by_model_group_name":
            group_name = required(arguments.get("model_name"), "model_name")
            return optional_row(router.get_deployment_by_model_group_name(group_name))
        case "get_model_ids":
            flag = arguments.get("exclude_team_models")
            if flag is None:
                raise ValueError("arguments.exclude_team_models is required")
            ids: list[JsonValue] = []
            ids.extend(router.get_model_ids(arguments.get("model_name"), exclude_team_models=flag))
            return ids
        case "has_model_id":
            return router.has_model_id(required(arguments.get("candidate_id"), "candidate_id"))
        case "resolve_model_name_from_model_id":
            return router.resolve_model_name_from_model_id(arguments.get("model_id"))
    raise ValueError(f"unsupported lookup function {call['function']}")


def listing_answer(router: Router, call: Input) -> JsonValue:
    arguments = call["arguments"]
    match call["function"]:
        case "get_candidate_model_ids_for_route":
            candidates: list[JsonValue] = []
            model = required(arguments.get("model_name"), "model_name")
            candidates.extend(sorted(router.get_candidate_model_ids_for_route(model, arguments.get("team_id"))))
            return candidates
        case "get_model_names":
            names: list[JsonValue] = []
            names.extend(router.get_model_names(arguments.get("team_id")))
            return names
        case "get_model_list":
            return listing_rows(router.get_model_list(arguments.get("model_name"), arguments.get("team_id")))
        case "get_model_list_from_model_alias":
            return listing_rows(router.get_model_list_from_model_alias(arguments.get("model_name")))
        case "should_include_deployment":
            index = arguments.get("deployment_index")
            if index is None:
                raise ValueError("arguments.deployment_index is required")
            model_name = required(arguments.get("model_name"), "model_name")
            return router.should_include_deployment(
                model_name=model_name, model=call["model_list"][index], team_id=arguments.get("team_id")
            )
    raise ValueError(f"unsupported listing function {call['function']}")


def run(input_value: Input) -> JsonValue:
    litellm.suppress_debug_info = True
    router = Router(
        model_list=input_value["model_list"],
        model_group_alias=input_value["model_group_alias"],
        routing_groups=input_value["routing_groups"],
    )
    if input_value["stale_index"]:
        router.model_id_to_deployment_index_map[STALE_ID] = STALE_INDEX
        router.model_name_to_deployment_indices[STALE_NAME] = [STALE_INDEX]
    if input_value["function"] in LOOKUP_FUNCTIONS:
        return lookup_answer(router, input_value)
    return listing_answer(router, input_value)
