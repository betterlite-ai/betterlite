from typing import NotRequired, TypedDict

import litellm
from litellm.responses.utils import ResponsesAPIRequestUtils
from litellm.router_utils.common_utils import resolve_model_group_alias

class Auth(TypedDict):
    team_alias: str | None
    key_alias: str | None


class Pipeline(TypedDict):
    policy_name: str
    mode: str
    guardrails: list[str]


class Input(TypedDict):
    function: str
    response_id: str | None
    router: bool
    deployments: dict[str, str]
    aliases: list[tuple[str, str]]
    model_group: str
    auth: Auth
    tags: list[str]
    pipelines: list[Pipeline]
    already_attached: list[str]


class Output(TypedDict):
    model_group: NotRequired[str | None]
    reason: NotRequired[str | None]
    hidden_by: NotRequired[str | None]
    attached: NotRequired[bool]
    team_alias: NotRequired[str | None]
    key_alias: NotRequired[str | None]
    model: NotRequired[str]
    tags: NotRequired[list[str] | None]
    pipelines: NotRequired[list[Pipeline]]
    managed_guardrails: NotRequired[list[str]]


def hidden_by(model_group: str, aliases: list[tuple[str, str]]) -> str | None:
    if "*" in model_group:
        return "a wildcard deployment"
    alias_map = dict(aliases)
    matching = [
        alias
        for alias, _ in aliases
        if resolve_model_group_alias(alias_map, alias) == model_group
    ]
    if not matching:
        return None
    return f"the target of model_group_alias {', '.join(matching)}"


def model_group_lookup(input_value: Input) -> Output:
    if not input_value["router"]:
        return {"model_group": None, "reason": "no router", "hidden_by": None}
    model_id = ResponsesAPIRequestUtils.get_model_id_from_response_id(
        input_value["response_id"]
    )
    if model_id is None:
        return {
            "model_group": None,
            "reason": "response id names no deployment",
            "hidden_by": None,
        }
    model_group = input_value["deployments"].get(model_id)
    if model_group is None:
        return {
            "model_group": None,
            "reason": "deployment no longer in the router",
            "hidden_by": None,
        }
    return {
        "model_group": model_group,
        "reason": None,
        "hidden_by": hidden_by(model_group, input_value["aliases"]),
    }


def retrieval_context(input_value: Input) -> Output:
    auth = input_value["auth"]
    tags = input_value["tags"]
    return {
        "team_alias": auth.get("team_alias"),
        "key_alias": auth.get("key_alias"),
        "model": input_value["model_group"],
        "tags": None if len(tags) == 0 else tags,
    }


def pipeline_filtering(input_value: Input) -> Output:
    attached = input_value["already_attached"]
    selected = [
        pipeline
        for pipeline in input_value["pipelines"]
        if pipeline["mode"] == "post_call" and pipeline["policy_name"] not in attached
    ]
    managed = sorted(
        {
            guardrail
            for pipeline in selected
            for guardrail in pipeline["guardrails"]
        }
    )
    return {"pipelines": selected, "managed_guardrails": managed}


def run(input_value: Input) -> Output:
    function = input_value["function"]
    if function == "model_group_lookup":
        return model_group_lookup(input_value)
    if function == "submit_model_hidden_by":
        return {"hidden_by": hidden_by(input_value["model_group"], input_value["aliases"])}
    if function == "retrieval_context":
        return retrieval_context(input_value)
    if function in {"pipeline_filtering", "deduplication"}:
        return pipeline_filtering(input_value)
    if function == "registry_not_initialized":
        return {"attached": False}
    raise ValueError(f"unknown {litellm.__name__} helper function {function}")
