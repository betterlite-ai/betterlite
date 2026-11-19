import logging
from typing import Final, Literal, TypedDict

import litellm
from litellm.router import Router
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

LOGGER: Final = "LiteLLM Router"
FIELDS: Final = (
    "key",
    "max_tokens",
    "max_input_tokens",
    "max_output_tokens",
    "input_cost_per_token",
    "output_cost_per_token",
    "litellm_provider",
)
AZURE_MARKERS: Final = ("has no base_model set", "Could not identify azure model")


class Arguments(TypedDict, total=False):
    model_name: str
    model_id: str
    id: str
    deployment: dict[str, JsonValue]
    received_model_name: str


class Input(TypedDict):
    function: Literal[
        "listing", "configured", "router_info", "azure_logs", "deployment_info", "raw"
    ]
    model_list: list[dict[str, JsonValue]]
    model_cost: dict[str, dict[str, JsonValue]]
    arguments: Arguments


class DeploymentRow(TypedDict):
    model_name: str
    litellm_params: dict[str, JsonValue]
    model_info: dict[str, JsonValue]


JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
ROW: Final[TypeAdapter[DeploymentRow | None]] = TypeAdapter(DeploymentRow | None)
ROWS: Final[TypeAdapter[list[DeploymentRow] | None]] = TypeAdapter(
    list[DeploymentRow] | None
)


class Capture(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.DEBUG)
        self.messages: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())


def model_id_of(arguments: Arguments) -> str:
    value = arguments.get("model_id")
    if value is None:
        raise ValueError("arguments.model_id is required")
    return value


def model_name_of(arguments: Arguments) -> str:
    value = arguments.get("model_name")
    if value is None:
        raise ValueError("arguments.model_name is required")
    return value


def received_of(arguments: Arguments) -> str:
    value = arguments.get("received_model_name")
    if value is None:
        raise ValueError("arguments.received_model_name is required")
    return value


def project_optional(info: object) -> dict[str, JsonValue] | None:
    if info is None:
        return None
    values = OBJECT.validate_python(info)
    return {name: values[name] for name in FIELDS if name in values}


def summary(row: DeploymentRow | None) -> dict[str, JsonValue] | None:
    if row is None:
        return None
    info = row["model_info"]
    params = row["litellm_params"]
    return {
        "id": info.get("id"),
        "model_name": row["model_name"],
        "model": params.get("model"),
    }


def listing_answer(router: Router, arguments: Arguments) -> JsonValue:
    info = router.get_model_listing_info(model_name_of(arguments))
    if info is None:
        return None
    return JSON.validate_python(
        {
            "cost_map_keys": list(info.cost_map_keys),
            "max_input_tokens": info.max_input_tokens,
            "max_output_tokens": info.max_output_tokens,
        }
    )


def configured_answer(router: Router, arguments: Arguments) -> JsonValue:
    name = model_name_of(arguments)
    limits = router.get_configured_token_limits(name)
    mode = router.get_configured_mode(name)
    display_name = router.get_configured_display_name(name)
    return JSON.validate_python(
        {"limits": [limits[0], limits[1]], "mode": mode, "display_name": display_name}
    )


def router_answer(router: Router, arguments: Arguments) -> JsonValue:
    name = received_of(arguments)
    deployment = arguments.get("deployment")
    if deployment is None:
        model_id = arguments.get("id")
        if model_id is None:
            raise ValueError("arguments.id is required")
        info = router.get_router_model_info(None, name, model_id)
    else:
        info = router.get_router_model_info(deployment, name)
    return JSON.validate_python(project_optional(info))


def deployment_answer(router: Router, arguments: Arguments) -> JsonValue:
    info = router.get_deployment_model_info(
        model_id_of(arguments), model_name_of(arguments)
    )
    return JSON.validate_python(project_optional(info))


def raw_answer(router: Router, arguments: Arguments) -> JsonValue:
    model_id = model_id_of(arguments)
    deployment = ROW.validate_python(router.get_model_info(model_id))
    group = ROWS.validate_python(router.get_model_group(model_id))
    found = None if group is None else [summary(row) for row in group]
    return JSON.validate_python({"deployment": summary(deployment), "group": found})


def azure_answer(router: Router, arguments: Arguments) -> JsonValue:
    deployment = arguments.get("deployment")
    if deployment is None:
        raise ValueError("arguments.deployment is required")
    capture = Capture()
    logger = logging.getLogger(LOGGER)
    kept_handlers = list(logger.handlers)
    previous_level = logger.level
    previous_propagate = logger.propagate
    logger.handlers = [capture]
    logger.propagate = False
    logger.setLevel(logging.DEBUG)
    try:
        info = router.get_router_model_info(deployment, received_of(arguments))
    finally:
        logger.handlers = kept_handlers
        logger.propagate = previous_propagate
        logger.setLevel(previous_level)
    logs = [
        message
        for message in capture.messages
        if any(marker in message for marker in AZURE_MARKERS)
    ]
    return JSON.validate_python({"logs": logs, "info": project_optional(info)})


def run(input_value: Input) -> JsonValue:
    litellm.suppress_debug_info = True
    router = Router(model_list=input_value["model_list"])
    litellm.model_cost = dict(input_value["model_cost"])
    function = input_value["function"]
    arguments = input_value["arguments"]
    if function == "azure_logs":
        return azure_answer(router, arguments)
    if function == "listing":
        return listing_answer(router, arguments)
    if function == "configured":
        return configured_answer(router, arguments)
    if function == "deployment_info":
        return deployment_answer(router, arguments)
    if function == "router_info":
        return router_answer(router, arguments)
    return raw_answer(router, arguments)
