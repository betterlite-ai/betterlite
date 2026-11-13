import logging
from typing import Final, TypedDict

from litellm.proxy.auth.auth_utils import (
    get_model_from_request,  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_utils.py#L1982
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
    body: dict[str, JsonValue]
    headers: dict[str, JsonValue]
    query: dict[str, JsonValue]
    team_id: str | None
    deployments: list[DeploymentDeclaration]
    endpoint_marker: bool | None
    path_params: list[str]
    fail: str | None


class Input(TypedDict):
    steps: list[Step]


INPUT: Final = TypeAdapter(Input)
OUTPUT: Final = TypeAdapter(dict[str, JsonValue])
PROXY_LOGGER: Final = "LiteLLM Proxy"


class MarkedEndpoint:
    __litellm_pass_through_endpoint__ = True


class PlainEndpoint:
    pass


class FailingResolverRouter(Router):
    def resolve_model_name_from_model_id(self, model_id: str | None) -> str | None:
        raise ValueError(f"the oracle resolver fails for {model_id}")


class DebugCapture(logging.Handler):
    def __init__(self) -> None:
        super().__init__()
        self.records: list[logging.LogRecord] = []

    def emit(self, record: logging.LogRecord) -> None:
        self.records.append(record)


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    typed = INPUT.validate_python(input_value)
    return [run_step(step) for step in typed["steps"]]


def run_step(step: Step) -> JsonValue:
    router = build_router(step)
    request = build_request(step)
    logger = logging.getLogger(PROXY_LOGGER)
    capture = DebugCapture()
    previous_level = logger.level
    logger.setLevel(logging.DEBUG)
    logger.addHandler(capture)
    try:
        pass_through = request_dispatched_to_pass_through_endpoint(request)
        provider_pass_through = request_dispatched_to_provider_pass_through(request)
        model = get_model_from_request(
            request_data=step["body"],
            route=step["route"],
            request_headers=step["headers"],
            request_query_params=step["query"],
            llm_router=router,
            request=request,
            team_id=step["team_id"],
        )
    finally:
        logger.removeHandler(capture)
        logger.setLevel(previous_level)
    debug = sum(1 for record in capture.records if record.levelno == logging.DEBUG)
    return OUTPUT.validate_python(
        {
            "model": model,
            "debug": debug,
            "pass_through": pass_through,
            "provider_pass_through": provider_pass_through,
        }
    )


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
    if step["fail"] == "resolver":
        return FailingResolverRouter(model_list=model_list)
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
