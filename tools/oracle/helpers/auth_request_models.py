from typing import Final, TypedDict

from litellm.proxy.auth.auth_utils import (
    get_cache_prediction_deployments,
    request_dispatched_to_pass_through_endpoint,
    request_dispatched_to_provider_pass_through,
)
from litellm.router import Router
from pydantic import JsonValue, TypeAdapter
from starlette.requests import Request
from starlette.types import Scope


class DeploymentDeclaration(TypedDict):
    model_name: str
    litellm_params: dict[str, JsonValue]
    model_info: dict[str, JsonValue]


class Step(TypedDict):
    route: str
    endpoint_marker: bool | None
    path_params: list[str]
    team_id: str | None
    deployments: list[DeploymentDeclaration]
    current_deployment_id: str | None
    candidate_deployment_id: str | None


class Input(TypedDict):
    steps: list[Step]


INPUT: Final = TypeAdapter(Input)
OUTPUT: Final = TypeAdapter(dict[str, JsonValue])


class MarkedEndpoint:
    __litellm_pass_through_endpoint__ = True


class PlainEndpoint:
    pass


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    typed = INPUT.validate_python(input_value)
    return [run_step(step) for step in typed["steps"]]


def run_step(step: Step) -> JsonValue:
    router = build_router(step)
    request = build_request(step)
    prediction = prediction_models(step, router)
    return OUTPUT.validate_python(
        {
            "prediction": prediction,
            "pass_through": request_dispatched_to_pass_through_endpoint(request),
            "provider_pass_through": request_dispatched_to_provider_pass_through(request),
        }
    )


def prediction_models(step: Step, router: Router | None) -> list[str] | None:
    current_id = step["current_deployment_id"]
    candidate_id = step["candidate_deployment_id"]
    if router is None or current_id is None or candidate_id is None:
        return None
    deployments = get_cache_prediction_deployments(
        current_deployment_id=current_id,
        candidate_deployment_id=candidate_id,
        llm_router=router,
        team_id=step["team_id"],
    )
    if deployments is None:
        return None
    return [deployments[0].model_name, deployments[1].model_name]


def build_router(step: Step) -> Router | None:
    if not step["deployments"]:
        return None
    model_list: list[dict[str, JsonValue]] = [
        {
            "model_name": deployment["model_name"],
            "litellm_params": deployment["litellm_params"],
            "model_info": deployment["model_info"],
        }
        for deployment in step["deployments"]
    ]
    return Router(model_list=model_list)


def build_request(step: Step) -> Request:
    path_params: dict[str, JsonValue] = {}
    for name in step["path_params"]:
        path_params[name] = "path-value"
    scope: Scope = {
        "type": "http",
        "path": step["route"],
        "path_params": path_params,
    }
    marker = step["endpoint_marker"]
    if marker is True:
        scope["endpoint"] = MarkedEndpoint()
    elif marker is False:
        scope["endpoint"] = PlainEndpoint()
    return Request(scope=scope)
