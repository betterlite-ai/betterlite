from decimal import Decimal
from typing import TypeAlias, TypedDict, cast

from litellm.litellm_core_utils.llm_cost_calc.tool_call_cost_tracking import (
    StandardBuiltInToolCostTracking as Tracker,
)
from litellm.litellm_core_utils.llm_cost_calc.utils import get_web_search_requests_from_usage
from litellm.types.llms.openai import ChatCompletionAnnotation, ResponsesAPIResponse
from litellm.types.responses.main import GenericResponseOutputItem
from litellm.types.utils import (
    Choices,
    Message,
    ModelResponse,
    PromptTokensDetailsWrapper,
    ServerToolUse,
    Usage,
)
from openai.types.responses.response_output_item import ResponseOutputItem

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class PromptDetailsInput(TypedDict):
    web_search_requests: int | None


class CostInput(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    prompt_details: PromptDetailsInput | None


class ServerToolUseInput(TypedDict, total=False):
    web_search_requests: int | None
    tool_search_requests: int | None
    browser_open_requests: int | None


class ToolUsageInput(TypedDict):
    cost: CostInput
    server_tool_use: ServerToolUseInput | None
    server_side_tool_usage_details: dict[str, Json] | None


class MessageInput(TypedDict):
    annotations: list[dict[str, Json]] | None


class ChoiceInput(TypedDict):
    message: MessageInput | None


class ChatInput(TypedDict):
    choices: list[ChoiceInput]
    usage: ToolUsageInput | None


class OutputItemInput(TypedDict, total=False):
    mapping: dict[str, Json]
    attribute: dict[str, Json]


class ResponsesInput(TypedDict):
    output: list[OutputItemInput]
    tool_usage: Json
    usage: ToolUsageInput | None


class RawInput(TypedDict):
    body: Json
    usage: ToolUsageInput | None


class OtherInput(TypedDict):
    usage: ToolUsageInput | None


class ResponseInput(TypedDict, total=False):
    chat: ChatInput
    responses: ResponsesInput
    raw: RawInput
    other: OtherInput


OutputItem: TypeAlias = dict[str, Json] | GenericResponseOutputItem
ResponseObject: TypeAlias = Json | ModelResponse | ResponsesAPIResponse


class Input(TypedDict):
    responses: list[ResponseInput]
    file_search_params: list[Json]
    token_counts: list[Json]
    values: list[Json]
    kwargs: list[dict[str, Json]]


def usage_record(value: ToolUsageInput | None) -> Usage | None:
    if value is None:
        return None
    cost = value["cost"]
    details = cost["prompt_details"]
    server_tool_use = value["server_tool_use"]
    prompt_details = (
        None
        if details is None
        else PromptTokensDetailsWrapper(web_search_requests=details["web_search_requests"])
    )
    tools = (
        None
        if server_tool_use is None
        else ServerToolUse(
            web_search_requests=server_tool_use.get("web_search_requests"),
            tool_search_requests=server_tool_use.get("tool_search_requests"),
            browser_open_requests=server_tool_use.get("browser_open_requests"),
        )
    )
    return Usage(
        prompt_tokens=cost["prompt_tokens"],
        completion_tokens=cost["completion_tokens"],
        total_tokens=cost["total_tokens"],
        prompt_tokens_details=prompt_details,
        server_tool_use=tools,
        server_side_tool_usage_details=value["server_side_tool_usage_details"],
    )


def choice_record(choice: ChoiceInput) -> Choices:
    message = choice["message"]
    built_message = Message(role="assistant", content="")
    built_message.annotations = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L734-L740
        "list[ChatCompletionAnnotation] | None",
        None if message is None else message["annotations"],
    )
    built_choice = Choices(finish_reason="stop", index=0, message=built_message)
    if message is None:
        del built_choice.message
    return built_choice


def chat_response(view: ChatInput) -> ModelResponse:
    response = ModelResponse()
    response.choices = [choice_record(choice) for choice in view["choices"]]
    return response


def output_item(entry: OutputItemInput) -> OutputItem:
    mapping = entry.get("mapping")
    if mapping is not None:
        return mapping
    attribute = entry.get("attribute")
    if attribute is None:
        raise ValueError("an output item names a mapping or an attribute")
    return GenericResponseOutputItem(
        type=cast(str, attribute["type"]),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/responses/main.py#L135-L147
        id="item_1",
        status="completed",
        role="assistant",
        content=[],
    )


def responses_response(view: ResponsesInput) -> ResponsesAPIResponse:
    # the base model allows extras, so tool_usage rides along beside the validated fields
    payload: dict[str, Json] = {
        "id": "resp_1",
        "created_at": 0,
        "output": [],
        "tool_usage": view["tool_usage"],
    }
    response = ResponsesAPIResponse.model_validate(payload)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/base.py#L29-L39
    response.output = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L1397-L1410
        "list[ResponseOutputItem | dict[str, Json]]",
        [output_item(entry) for entry in view["output"]],
    )
    return response


def response_object(view: ResponseInput) -> ResponseObject:
    chat = view.get("chat")
    if chat is not None:
        return chat_response(chat)
    responses = view.get("responses")
    if responses is not None:
        return responses_response(responses)
    raw = view.get("raw")
    if raw is not None:
        return raw["body"]
    return None


def response_usage(view: ResponseInput) -> Usage | None:
    chat = view.get("chat")
    if chat is not None:
        return usage_record(chat["usage"])
    responses = view.get("responses")
    if responses is not None:
        return usage_record(responses["usage"])
    raw = view.get("raw")
    if raw is not None:
        return usage_record(raw["usage"])
    other = view.get("other")
    if other is not None:
        return usage_record(other["usage"])
    return None


def usage_value(usage: Usage | None) -> Json:
    if usage is None:
        return None
    prompt_details = usage.prompt_tokens_details
    return {
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
        "prompt_web_search_requests": (
            None if prompt_details is None else prompt_details.web_search_requests
        ),
        "server_tool_use": get_web_search_requests_from_usage(usage),
    }


def response_value(view: ResponseInput) -> Json:
    response = response_object(view)
    usage = response_usage(view)
    count = Tracker._count_web_search_calls(response)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/llm_cost_calc/tool_call_cost_tracking.py#L193
    repaired = Tracker._usage_with_anthropic_web_search(usage, response)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/llm_cost_calc/tool_call_cost_tracking.py#L382
    return {
        "web_search": Tracker.response_object_includes_web_search_call(
            response_object=response, usage=usage
        ),
        "file_search": Tracker.response_object_includes_file_search_call(
            response_object=response
        ),
        "count": count,
        "usage": usage_value(repaired),
    }


def decimal_json(value: float | None) -> Json:
    if value is None:
        return None
    if not value:
        return "0"
    return format(Decimal(repr(value)).normalize(), "f")


def file_search_value(value: Json) -> Json:
    storage_gb, days = Tracker._extract_file_search_params(value)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/llm_cost_calc/tool_call_cost_tracking.py#L265
    return [decimal_json(storage_gb), decimal_json(days)]


def token_count_value(value: Json) -> Json:
    input_tokens, output_tokens = Tracker._extract_token_counts(value)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/llm_cost_calc/tool_call_cost_tracking.py#L355
    return [input_tokens, output_tokens]


def kwargs_value(kwargs: dict[str, Json]) -> Json:
    options = Tracker._get_web_search_options(kwargs)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/llm_cost_calc/tool_call_cost_tracking.py#L742
    tool = Tracker._get_file_search_tool_call(kwargs)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/llm_cost_calc/tool_call_cost_tracking.py#L764
    return cast(Json, {"web_search_options": options, "file_search": tool})  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L701-L717


def run(input_value: Input) -> Json:
    return {
        "responses": [response_value(view) for view in input_value["responses"]],
        "file_search_params": [
            file_search_value(value) for value in input_value["file_search_params"]
        ],
        "token_counts": [token_count_value(value) for value in input_value["token_counts"]],
        "values": [
            Tracker._safe_convert_to_int(value) for value in input_value["values"]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/llm_cost_calc/tool_call_cost_tracking.py#L372
        ],
        "kwargs": [kwargs_value(value) for value in input_value["kwargs"]],
    }
