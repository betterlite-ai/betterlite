import json
from typing import Final, Literal, NotRequired, Protocol, TypedDict

import litellm
from litellm.router import Router
from litellm.router_utils import pattern_match_deployments
from litellm.types.router import Deployment, RouterModelGroupAliasItem, RoutingGroup
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
    missing_ids: NotRequired[list[int]]
    missing_info: NotRequired[list[int]]
    arguments: Arguments


class ParamsView(TypedDict):
    model: str


class DeploymentView(TypedDict):
    model_name: str
    litellm_params: ParamsView
    model_info: NotRequired[dict[str, JsonValue]]


class StoredRow(TypedDict, total=False):
    model_info: dict[str, JsonValue]


class RouterStorage(Protocol):
    model_list: list[StoredRow]


class FixedProvider:
    def __init__(self, provider: str | None) -> None:
        self.provider = provider

    def __call__(self, model: str | None) -> str | None:  # noqa: ARG002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/get_llm_provider_logic.py#L146
        return self.provider


GROUPS: Final[TypeAdapter[list[RoutingGroup | dict[str, JsonValue]]]] = TypeAdapter(
    list[RoutingGroup | dict[str, JsonValue]]
)
VIEW: Final[TypeAdapter[DeploymentView]] = TypeAdapter(DeploymentView)
LISTING: Final[TypeAdapter[list[DeploymentView] | None]] = TypeAdapter(list[DeploymentView] | None)


def required(value: str | None, name: str) -> str:
    if value is None:
        raise ValueError(f"arguments.{name} is required")
    return value


def row_view(value: DeploymentView) -> dict[str, JsonValue]:
    info: dict[str, JsonValue] = {}
    declared = value.get("model_info")
    if declared is not None:
        info = declared
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


def listing_rows(rows: object) -> list[JsonValue]:
    listed = LISTING.validate_python(rows)
    if listed is None:
        return []
    return [row_view(view) for view in listed]


def lookup_answer(router: Router, call: Input) -> JsonValue:
    arguments = call["arguments"]
    match call["function"]:
        case "get_deployment":
            given = required(arguments.get("model_id"), "model_id")
            return optional_row(router.get_deployment(given))
        case "get_deployment_by_model_group_name":
            group_name = required(arguments.get("model_name"), "model_name")
            return optional_row(router.get_deployment_by_model_group_name(group_name))
        case "get_model_ids":
            flag = arguments.get("exclude_team_models")
            exclude = False if flag is None else flag
            ids: list[JsonValue] = []
            ids.extend(
                router.get_model_ids(arguments.get("model_name"), exclude_team_models=exclude)
            )
            return ids
        case "has_model_id":
            candidate = required(arguments.get("candidate_id"), "candidate_id")
            return router.has_model_id(candidate)
        case "resolve_model_name_from_model_id":
            return router.resolve_model_name_from_model_id(arguments.get("model_id"))
        case _:
            raise ValueError(f"unsupported lookup function {call['function']}")


def listing_answer(router: Router, call: Input) -> JsonValue:
    arguments = call["arguments"]
    match call["function"]:
        case "get_candidate_model_ids_for_route":
            model = required(arguments.get("model_name"), "model_name")
            candidates: list[JsonValue] = []
            candidates.extend(
                sorted(router.get_candidate_model_ids_for_route(model, arguments.get("team_id")))
            )
            return candidates
        case "get_model_names":
            names: list[JsonValue] = []
            names.extend(router.get_model_names(arguments.get("team_id")))
            return names
        case "get_model_list":
            listed: object = router.get_model_list(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L12041-L12048
                arguments.get("model_name"), arguments.get("team_id")
            )
            return listing_rows(listed)
        case "get_model_list_from_model_alias":
            aliased: object = router.get_model_list_from_model_alias(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L11851-L11857
                arguments.get("model_name")
            )
            return listing_rows(aliased)
        case "should_include_deployment":
            index = arguments.get("deployment_index")
            if index is None:
                raise ValueError("arguments.deployment_index is required")
            return router.should_include_deployment(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L11618
                model_name=required(arguments.get("model_name"), "model_name"),
                model=call["model_list"][index],
                team_id=arguments.get("team_id"),
            )
        case _:
            raise ValueError(f"unsupported listing function {call['function']}")


def run(input_value: Input) -> JsonValue:
    litellm.suppress_debug_info = True
    groups = GROUPS.validate_json(json.dumps(input_value["routing_groups"]))
    router = Router(
        model_list=input_value["model_list"],
        model_group_alias=input_value["model_group_alias"],
        routing_groups=groups,
    )
    if input_value["stale_index"]:
        router.model_id_to_deployment_index_map[STALE_ID] = STALE_INDEX
        router.model_name_to_deployment_indices[STALE_NAME] = [STALE_INDEX]
    storage: RouterStorage = router
    missing_ids = input_value.get("missing_ids")
    if missing_ids is not None:
        for index in missing_ids:
            info = storage.model_list[index].get("model_info")  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L1053
            if info is not None:
                del info["id"]
    missing_info = input_value.get("missing_info")
    if missing_info is not None:
        for index in missing_info:
            del storage.model_list[index]["model_info"]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L1053
    provider = pattern_match_deployments.inferred_provider  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router_utils/pattern_match_deployments.py#L11
    pattern_match_deployments.inferred_provider = FixedProvider(input_value["inferred_provider"])  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router_utils/pattern_match_deployments.py#L11
    try:
        if input_value["function"] in LOOKUP_FUNCTIONS:
            return lookup_answer(router, input_value)
        return listing_answer(router, input_value)
    finally:
        pattern_match_deployments.inferred_provider = provider  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router_utils/pattern_match_deployments.py#L11
