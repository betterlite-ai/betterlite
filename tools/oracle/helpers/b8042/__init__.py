from collections.abc import Callable
from typing import Literal, Protocol, TypeAlias, TypedDict, cast

from litellm.proxy.types_utils.utils import (
    get_instance_fn,
    validate_custom_validate_return_type,
)

Fixture: TypeAlias = Literal["literal_true", "bool_annotation", "no_annotation", "none_argument"]
Validator: TypeAlias = Callable[[], bool] | Callable[[], None]


class LoadInput(TypedDict):
    op: Literal["load"]
    value: str
    config_file_path: str | None


class ReturnInput(TypedDict):
    op: Literal["check_return"]
    fixture: Fixture


Input: TypeAlias = LoadInput | ReturnInput


class Loaded(Protocol):
    __module__: str
    __qualname__: str


class FailureResult(TypedDict):
    ok: Literal[False]
    type: str
    text: str


class LoadedResult(TypedDict):
    ok: Literal[True]
    module: str
    qualname: str


class CheckedResult(TypedDict):
    ok: Literal[True]
    result: Literal["function", "none"]


Result: TypeAlias = FailureResult | LoadedResult | CheckedResult


def validator() -> Literal[True]:
    accepted: Literal[True] = True
    return accepted


def bool_validator() -> bool:
    accepted: bool = True
    return accepted


def fixture_function(name: Fixture) -> Validator | None:
    if name == "literal_true":
        return validator
    if name == "bool_annotation":
        return bool_validator
    if name == "no_annotation":
        return print
    return None


def failure(error: Exception) -> FailureResult:
    described: FailureResult = {"ok": False, "type": type(error).__name__, "text": str(error)}
    return described


def load_result(value: str, config_file_path: str | None) -> Result:
    try:
        loaded = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/types_utils/utils.py#L9
            Loaded, get_instance_fn(value, config_file_path=config_file_path)
        )
    except Exception as error:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/types_utils/utils.py#L9-L66
        return failure(error)
    described: LoadedResult = {
        "ok": True,
        "module": loaded.__module__,
        "qualname": loaded.__qualname__,
    }
    return described


def check_return(name: Fixture) -> Result:
    function = fixture_function(name)
    try:
        checked = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/types_utils/utils.py#L200-L212
            "Callable[[], bool] | None", validate_custom_validate_return_type(function)
        )
    except Exception as error:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/types_utils/utils.py#L200-L212
        return failure(error)
    if checked is None:
        none_result: CheckedResult = {"ok": True, "result": "none"}
        return none_result
    function_result: CheckedResult = {"ok": True, "result": "function"}
    return function_result


def run(input_value: Input) -> Result:
    if input_value["op"] == "load":
        return load_result(input_value["value"], input_value["config_file_path"])
    return check_return(input_value["fixture"])
