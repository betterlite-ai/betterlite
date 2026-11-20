from typing import TypedDict

import litellm
from litellm.proxy.auth.auth_utils import is_request_body_safe  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_utils.py#L477-L539
from litellm.router import Router
from pydantic import JsonValue


class Deployment(TypedDict):
    model_name: str
    litellm_params: dict[str, JsonValue]


class Input(TypedDict):
    request_body: dict[str, JsonValue]
    general_settings: dict[str, JsonValue]
    model: str
    model_list: list[Deployment]


def run(input_value: Input) -> bool:
    litellm.suppress_debug_info = True
    return is_request_body_safe(
        request_body=input_value["request_body"],
        general_settings=input_value["general_settings"],
        llm_router=build_router(input_value["model_list"]),
        model=input_value["model"],
    )


def build_router(deployments: list[Deployment]) -> Router | None:
    if not deployments:
        return None
    model_list: list[dict[str, JsonValue]] = [
        {
            "model_name": deployment["model_name"],
            "litellm_params": deployment["litellm_params"],
        }
        for deployment in deployments
    ]
    return Router(model_list=model_list)
