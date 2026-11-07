from collections.abc import Callable
from typing import Annotated, Literal, cast

import litellm
from litellm.router_utils.pattern_match_deployments import PatternMatchRouter
from pydantic import Field
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

AddPattern = Callable[[str, dict[str, object]], None]
MatchDeployments = Callable[[str, list[str] | None], list[dict[str, object]] | None]
GetPatterns = Callable[[str, str | None], list[dict[str, object]] | None]
GetDeploymentList = Callable[[str, str | None], list[dict[str, object]]]


class Add(Strict):
    action: Literal["add"]
    pattern: str
    model: str
    model_id: str | None = None


class Remove(Strict):
    action: Literal["remove"]
    model_id: str


class RouteStep(Strict):
    action: Literal["route"]
    request: str
    filtered_model_names: list[str] | None = None


class GetPattern(Strict):
    action: Literal["get_pattern"]
    model: str
    provider: str | None = None


class GetDeployments(Strict):
    action: Literal["get_deployments"]
    model: str
    provider: str | None = None


class Resolved(Strict):
    model: str
    model_id: str | None


class Output(Strict):
    routes: list[list[Resolved] | None]


def deployment(step: Add) -> dict[str, object]:
    model_info: dict[str, object] = {}
    if step.model_id is not None:
        model_info["id"] = step.model_id
    return {"litellm_params": {"model": step.model}, "model_info": model_info}


def resolved(value: dict[str, object]) -> Resolved:
    params = value.get("litellm_params")
    if not isinstance(params, dict):
        raise ProbeError("invalid_request", "a routed deployment must have litellm_params")
    model: object = cast("dict[str, object]", params).get("model")
    if not isinstance(model, str):
        raise ProbeError("invalid_request", "a routed deployment must have a model")
    info = value.get("model_info")
    model_id: str | None = None
    if isinstance(info, dict):
        identifier: object = cast("dict[str, object]", info).get("id")
        if isinstance(identifier, str):
            model_id = identifier
    return Resolved(model=model, model_id=model_id)


def matched(deployments: list[dict[str, object]] | None) -> list[Resolved] | None:
    if deployments is None:
        return None
    return [resolved(value) for value in deployments]


class WildcardCatalog(Operation):
    steps: list[
        Annotated[
            Add | Remove | RouteStep | GetPattern | GetDeployments,
            Field(discriminator="action"),
        ]
    ]

    @override
    def run(self) -> Output:
        # an unmapped model name makes get_llm_provider print the provider list to stdout, where the reply goes
        litellm.suppress_debug_info = True
        router = PatternMatchRouter()
        add_pattern = cast("AddPattern", router.add_pattern)
        remove_deployment = cast("Callable[[str], None]", router.remove_deployment)
        route = cast("MatchDeployments", router.route)
        get_pattern = cast("GetPatterns", router.get_pattern)
        get_deployments = cast("GetDeploymentList", router.get_deployments_by_pattern)
        routes: list[list[Resolved] | None] = []
        for step in self.steps:
            if isinstance(step, Add):
                add_pattern(step.pattern, deployment(step))
            elif isinstance(step, Remove):
                remove_deployment(step.model_id)
            elif isinstance(step, RouteStep):
                routes.append(matched(route(step.request, step.filtered_model_names)))
            elif isinstance(step, GetPattern):
                routes.append(matched(get_pattern(step.model, step.provider)))
            else:
                routes.append(matched(get_deployments(step.model, step.provider)))
        return Output(routes=routes)
