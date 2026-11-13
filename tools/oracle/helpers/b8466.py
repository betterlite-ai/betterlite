from types import NoneType
from typing import Literal, Required, TypedDict

from litellm.router import Router
from pydantic import JsonValue

LITELLM_AGENT_PREFIX = "litellm_agent/"
PROMPT_MANAGEMENT_PARAMS = (
    "bitbucket_config",
    "dotprompt_config",
    "prompt_id",
    "prompt_variables",
    "prompt_label",
    "prompt_version",
)


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


def _object(value: JsonValue, name: str) -> dict[str, JsonValue]:
    match value:
        case dict():
            return value
        case NoneType() | bool() | int() | float() | str() | list():
            raise ValueError(f"{name} must be an object")


def _array(value: JsonValue, name: str) -> list[JsonValue]:
    match value:
        case list():
            return value
        case NoneType() | bool() | int() | float() | str() | dict():
            raise ValueError(f"{name} must be a list")


def _string(value: JsonValue, name: str) -> str:
    match value:
        case str(text):
            return text
        case NoneType() | bool() | int() | float() | list() | dict():
            raise ValueError(f"{name} must be a string")


def _string_or_none(value: JsonValue) -> str | None:
    match value:
        case str(text):
            return text
        case NoneType() | bool() | int() | float() | list() | dict():
            return None


def _is_object(value: JsonValue) -> bool:
    match value:
        case dict():
            return True
        case NoneType() | bool() | int() | float() | str() | list():
            return False


def _python_type(value: JsonValue) -> str:
    match value:
        case NoneType():
            return "NoneType"
        case bool():
            return "bool"
        case int():
            return "int"
        case float():
            return "float"
        case str():
            return "str"
        case list():
            return "list"
        case dict():
            return "dict"


def _prefer(
    request: dict[str, JsonValue], data: dict[str, JsonValue], key: str
) -> JsonValue:
    requested = request.get(key)
    if requested:
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
    if not agent and _string_or_none(prompt_id) is None:
        raise ValueError(
            f"Prompt ID is not set or not a string. Got={prompt_id},"
            f" type=<class '{_python_type(prompt_id)}'>"
        )
    if prompt_variables is not None and not _is_object(prompt_variables):
        raise ValueError(
            f"Prompt variables is set but not a dictionary. Got={prompt_variables},"
            f" type=<class '{_python_type(prompt_variables)}'>"
        )
    return {
        "prompt_id": prompt_id,
        "prompt_variables": prompt_variables,
        "prompt_label": prompt_label,
    }


def _is_prompt_model(input_value: Input) -> bool:
    router = _PromptRouter(model_list=input_value["model_list"])
    return router.is_prompt_model(input_value["model"])


def _identity_output(input_value: Input) -> dict[str, JsonValue]:
    request = input_value["request"]
    data = _object(input_value["deployment"]["litellm_params"], "deployment")
    identity = _resolve_identity(request, data)
    return {
        "prompt_id": identity["prompt_id"],
        "prompt_variables": identity["prompt_variables"],
        "prompt_label": identity["prompt_label"],
    }


def _merge_output(input_value: Input) -> dict[str, JsonValue]:
    request = input_value["request"]
    data = _object(input_value["deployment"]["litellm_params"], "deployment")
    identity = _resolve_identity(request, data)
    compiled = input_value["compiled"]
    filtered = {
        key: value for key, value in data.items() if key not in PROMPT_MANAGEMENT_PARAMS
    }
    optional = _object(compiled["optional_params"], "compiled.optional_params")
    merged = {**filtered, **request, **optional}
    merged["model"] = _string(compiled["model"], "compiled.model")
    merged["messages"] = _array(compiled["messages"], "compiled.messages")
    logging = request.get("litellm_logging_obj")
    if logging is not None:
        merged["litellm_logging_obj"] = logging
    merged["prompt_id"] = identity["prompt_id"]
    merged["prompt_variables"] = identity["prompt_variables"]
    merged["prompt_label"] = identity["prompt_label"]
    if input_value["new_model_has_deployments"]:
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
