from typing import Literal, TypeAlias, TypedDict

import litellm
from litellm.proxy.proxy_server import (
    pin_complexity_router_model_id,  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/proxy_server.py#L5116
    validate_auto_router_capability_limits,
    validate_deployment_access_windows,
    validate_deployment_complexity_router_placement,
    validate_deployment_max_agentic_loops,
)
from pydantic import JsonValue

litellm.suppress_debug_info = True

ModelOperation: TypeAlias = Literal[
    "max_agentic_loops",
    "complexity_placement",
    "access_windows",
]


class Input(TypedDict):
    operation: Literal[
        "max_agentic_loops",
        "complexity_placement",
        "access_windows",
        "capability_limits",
        "pin_model_id",
    ]
    model: dict[str, JsonValue]
    model_list: list[dict[str, JsonValue]]
    limit: int | None


def model_refusal(
    model: dict[str, JsonValue], operation: ModelOperation
) -> dict[str, JsonValue]:
    try:
        match operation:
            case "max_agentic_loops":
                validate_deployment_max_agentic_loops(model)
            case "complexity_placement":
                validate_deployment_complexity_router_placement(model)
            case "access_windows":
                validate_deployment_access_windows(model)
    except (ValueError, TypeError) as error:
        return {
            "refused": True,
            "model_name": model.get("model_name"),
            "message": str(error),
        }
    return {"refused": False, "model_name": model.get("model_name"), "message": None}


def capability_limits(
    model_list: list[dict[str, JsonValue]], limit: int | None
) -> dict[str, JsonValue]:
    try:
        validate_auto_router_capability_limits(model_list, limit=limit)
    except ValueError as error:
        return {"refused": True, "message": str(error)}
    return {"refused": False, "message": None}


def pinned_model(model: dict[str, JsonValue]) -> dict[str, JsonValue]:
    pin_complexity_router_model_id(model)
    return model


def run(input_value: Input) -> JsonValue:
    match input_value["operation"]:
        case "max_agentic_loops":
            return model_refusal(input_value["model"], "max_agentic_loops")
        case "complexity_placement":
            return model_refusal(input_value["model"], "complexity_placement")
        case "access_windows":
            return model_refusal(input_value["model"], "access_windows")
        case "capability_limits":
            return capability_limits(input_value["model_list"], input_value["limit"])
        case "pin_model_id":
            return pinned_model(input_value["model"])
