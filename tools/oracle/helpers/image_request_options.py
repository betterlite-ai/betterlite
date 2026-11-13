from typing import Literal, TypedDict

from litellm.litellm_core_utils.llm_cost_calc import utils
from litellm.litellm_core_utils.llm_cost_calc.utils import CostCalculatorUtils
from pydantic import JsonValue


class SizeCall(TypedDict):
    function: Literal["size"]
    optional_params: dict[str, JsonValue] | None


class ParamCall(TypedDict):
    function: Literal["param"]
    key: str
    optional_params: dict[str, JsonValue] | None


class CallTypeCall(TypedDict):
    function: Literal["call_type"]
    call_type: str


Call = SizeCall | ParamCall | CallTypeCall


class Input(TypedDict):
    calls: list[Call]


def run_call(call: Call) -> JsonValue:
    match call["function"]:
        case "size":
            return utils._requested_image_size(call["optional_params"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/llm_cost_calc/utils.py#L121-L123
        case "param":
            return utils._requested_image_param(call["optional_params"], call["key"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/llm_cost_calc/utils.py#L116-L118
        case "call_type":
            return CostCalculatorUtils._call_type_has_image_response(call["call_type"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/llm_cost_calc/utils.py#L1900-L1909


def run(input_value: Input) -> JsonValue:
    return [
        run_call(call)
        for call in input_value["calls"]
    ]
