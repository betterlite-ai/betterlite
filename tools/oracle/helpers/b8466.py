from typing import Literal, Required, TypedDict

from litellm.router import Router
from pydantic import JsonValue, TypeAdapter

LITELLM_AGENT_PREFIX = "litellm_agent/"
PROMPT_MANAGEMENT_PARAMS = (
    "bitbucket_config",
    "dotprompt_config",
    "prompt_id",
    "prompt_variables",
    "prompt_label",
    "prompt_version",
)

OBJECT: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
ARRAY: TypeAdapter[list[JsonValue]] = TypeAdapter(list[JsonValue])
ROWS: TypeAdapter[list[dict[str, JsonValue]]] = TypeAdapter(list[dict[str, JsonValue]])
STRING: TypeAdapter[str] = TypeAdapter(str)
BOOLEAN: TypeAdapter[bool] = TypeAdapter(bool)


class Identity(TypedDict):
    prompt_id: JsonValue
    prompt_variables: JsonValue
    prompt_label: JsonValue


class Input(TypedDict, total=False):
    function: Required[
        Literal["is_prompt_model", "resolve_prompt_identity", "merge_prompt_request"]
    ]
    model: str
    model_list: list[dict[str, JsonValue]]
    request: dict[str, JsonValue]
    deployment: dict[str, JsonValue]
    compiled: dict[str, JsonValue]
    new_model_has_deployments: bool


class _PromptRouter(Router):
    def is_prompt_model(self, model: str) -> bool:
        return self._is_prompt_management_model(model)


def _string_or_none(value: JsonValue) -> str | None:
    if value is None:
        return None
    match value:
        case str(text):
            return text
        case bool() | int() | float() | list() | dict():
            return None


def _prefer(
    request: dict[str, JsonValue], data: dict[str, JsonValue], key: str
) -> JsonValue:
    requested = request.get(key)
    if requested is not None:
        return requested
    return data.get(key)


def _resolve_identity(
    request: dict[str, JsonValue], data: dict[str, JsonValue]
) -> Identity:
    prompt_id = _prefer(request, data, "prompt_id")
    prompt_variables = _prefer(request, data, "prompt_variables")
    prompt_label = _prefer(request, data, "prompt_label")
    model = _string_or_none(data.get("model"))
    agent = model is not None and model.startswith(LITELLM_AGENT_PREFIX)
    if not agent:
        STRING.validate_python(prompt_id)
    if prompt_variables is not None:
        OBJECT.validate_python(prompt_variables)
    return {
        "prompt_id": prompt_id,
        "prompt_variables": prompt_variables,
        "prompt_label": prompt_label,
    }


def _is_prompt_model(input_value: Input) -> bool:
    model = STRING.validate_python(input_value.get("model"))
    rows = ROWS.validate_python(input_value.get("model_list"))
    router = _PromptRouter(model_list=rows)
    return router.is_prompt_model(model)


def _identity_output(input_value: Input) -> dict[str, JsonValue]:
    request = OBJECT.validate_python(input_value.get("request"))
    deployment = OBJECT.validate_python(input_value.get("deployment"))
    data = OBJECT.validate_python(deployment.get("litellm_params"))
    identity = _resolve_identity(request, data)
    return {
        "prompt_id": identity["prompt_id"],
        "prompt_variables": identity["prompt_variables"],
        "prompt_label": identity["prompt_label"],
    }


def _merge_output(input_value: Input) -> dict[str, JsonValue]:
    request = OBJECT.validate_python(input_value.get("request"))
    deployment = OBJECT.validate_python(input_value.get("deployment"))
    data = OBJECT.validate_python(deployment.get("litellm_params"))
    identity = _resolve_identity(request, data)
    compiled = OBJECT.validate_python(input_value.get("compiled"))
    optional = OBJECT.validate_python(compiled.get("optional_params"))
    filtered = {
        key: value for key, value in data.items() if key not in PROMPT_MANAGEMENT_PARAMS
    }
    merged = {**filtered, **request, **optional}
    merged["model"] = STRING.validate_python(compiled.get("model"))
    merged["messages"] = ARRAY.validate_python(compiled.get("messages"))
    logging = request.get("litellm_logging_obj")
    if logging is not None:
        merged["litellm_logging_obj"] = logging
    merged["prompt_id"] = identity["prompt_id"]
    merged["prompt_variables"] = identity["prompt_variables"]
    merged["prompt_label"] = identity["prompt_label"]
    if BOOLEAN.validate_python(input_value.get("new_model_has_deployments")):
        return {"request": merged, "call": "with_fallbacks"}
    merged.pop("original_function", None)
    return {"request": merged, "call": "direct"}


def run(input_value: Input) -> dict[str, JsonValue]:
    match input_value["function"]:
        case "is_prompt_model":
            return {"is_prompt_model": _is_prompt_model(input_value)}
        case "resolve_prompt_identity":
            return _identity_output(input_value)
        case "merge_prompt_request":
            return _merge_output(input_value)
