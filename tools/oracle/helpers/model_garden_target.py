from typing import TypedDict

import litellm
from litellm.llms.vertex_ai.common_utils import (
    get_vertex_base_model_name,
    vertex_model_garden_model_id_in_json_body,
)
from litellm.llms.vertex_ai.vertex_model_garden.main import create_vertex_url

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True

DEFAULT_LOCATION = "us-central1"


class Target(TypedDict):
    model: str
    api_base: str | None
    vertex_location: str | None
    vertex_project: str | None
    resolved_project: str


class TargetResult(TypedDict):
    api_base: str
    model: str


class Input(TypedDict):
    targets: list[Target]


class Output(TypedDict):
    targets: list[TargetResult]


def target_of(target: Target) -> TargetResult:
    model = get_vertex_base_model_name(target["model"])
    api_base = target["api_base"]
    if api_base is None:
        api_base = create_vertex_url(
            vertex_location=target["vertex_location"] or DEFAULT_LOCATION,
            vertex_project=target["vertex_project"] or target["resolved_project"],
            stream=None,
            model=model,
        )
    return TargetResult(
        api_base=api_base,
        model=model if vertex_model_garden_model_id_in_json_body(model) else "",
    )


def run(input_value: Input) -> Output:
    targets = [target_of(target) for target in input_value["targets"]]
    return Output(targets=targets)
