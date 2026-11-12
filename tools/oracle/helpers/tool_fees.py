from decimal import Decimal
from typing import TypeAlias, TypedDict, cast

from litellm.litellm_core_utils.llm_cost_calc.tool_call_cost_tracking import (
    StandardBuiltInToolCostTracking as Tracker,
)
from litellm.types.llms.openai import FileSearchTool, WebSearchOptions
from litellm.types.utils import ModelInfo

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Quantity: TypeAlias = int | float | None
ModelInfoInput: TypeAlias = dict[str, Json] | None


class WebSearchCall(TypedDict):
    options: dict[str, Json] | None
    model_info: ModelInfoInput


class FileSearchCall(TypedDict):
    file_search: dict[str, Json] | None
    provider: str | None
    model_info: ModelInfoInput
    storage_gb: Quantity
    days: Quantity


class VectorStoreCall(TypedDict):
    usage: dict[str, float]
    provider: str | None
    model_info: ModelInfoInput


class ComputerUseCall(TypedDict):
    input_tokens: int | None
    output_tokens: int | None
    provider: str | None
    model_info: ModelInfoInput


class CodeInterpreterCall(TypedDict):
    sessions: int | None
    provider: str | None
    model_info: ModelInfoInput


class Input(TypedDict):
    web_search: list[WebSearchCall]
    file_search: list[FileSearchCall]
    vector_store: list[VectorStoreCall]
    computer_use: list[ComputerUseCall]
    code_interpreter: list[CodeInterpreterCall]


def decimal_json(value: float | None) -> Json:
    if value is None:
        return None
    if not value:
        return "0"
    return format(Decimal(repr(value)).normalize(), "f")


def web_search_value(call: WebSearchCall) -> Json:
    options = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L701-L717
        "WebSearchOptions | None", call["options"]
    )
    model_info = cast(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L426-L430
        "ModelInfo | None", call["model_info"]
    )
    cost = Tracker.get_cost_for_web_search(options, model_info)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/llm_cost_calc/tool_call_cost_tracking.py#L532-L536
    return decimal_json(cost)


def file_search_value(call: FileSearchCall) -> Json:
    tool = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L712-L716
        "FileSearchTool | None", call["file_search"]
    )
    cost = Tracker.get_cost_for_file_search(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/llm_cost_calc/tool_call_cost_tracking.py#L569-L577
        tool, call["provider"], call["model_info"], call["storage_gb"], call["days"]
    )
    return decimal_json(cost)


def vector_store_value(call: VectorStoreCall) -> Json:
    cost = Tracker.get_cost_for_vector_store(call["usage"], call["provider"], call["model_info"])  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/llm_cost_calc/tool_call_cost_tracking.py#L605-L610
    return decimal_json(cost)


def computer_use_value(call: ComputerUseCall) -> Json:
    cost = Tracker.get_cost_for_computer_use(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/llm_cost_calc/tool_call_cost_tracking.py#L635-L641
        call["input_tokens"], call["output_tokens"], call["provider"], call["model_info"]
    )
    return decimal_json(cost)


def code_interpreter_value(call: CodeInterpreterCall) -> Json:
    cost = Tracker.get_cost_for_code_interpreter(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/llm_cost_calc/tool_call_cost_tracking.py#L698-L702
        call["sessions"], call["provider"], call["model_info"]
    )
    return decimal_json(cost)


def run(input_value: Input) -> Json:
    return {
        "web_search": [web_search_value(call) for call in input_value["web_search"]],
        "file_search": [file_search_value(call) for call in input_value["file_search"]],
        "vector_store": [vector_store_value(call) for call in input_value["vector_store"]],
        "computer_use": [computer_use_value(call) for call in input_value["computer_use"]],
        "code_interpreter": [
            code_interpreter_value(call) for call in input_value["code_interpreter"]
        ],
    }
