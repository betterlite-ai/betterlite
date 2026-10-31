from typing import TypeAlias, cast

from litellm.litellm_core_utils.prompt_templates import common_utils
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import ModelResponse

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    operation = cast(str, input_value["operation"])
    if operation == "convert_content_list_to_str":
        result = common_utils.convert_content_list_to_str(
            cast(AllMessageValues, input_value["message"])
        )
    elif operation == "extract_search_results_text":
        result = common_utils.extract_search_results_text(input_value.get("search_results"))
    elif operation == "request_contains_image_content":
        result = common_utils.request_contains_image_content(
            cast(list[dict[str, object]], input_value["messages"])
        )
    elif operation == "convert_openai_message_to_only_content_messages":
        result = common_utils.convert_openai_message_to_only_content_messages(
            cast(list[AllMessageValues], input_value["messages"])
        )
    elif operation == "text_completion_prompt_to_messages":
        result = common_utils.text_completion_prompt_to_messages(input_value.get("prompt"))
    elif operation == "handle_any_messages_to_chat_completion_str_messages_conversion":
        result = common_utils.handle_any_messages_to_chat_completion_str_messages_conversion(
            input_value.get("any_messages")
        )
    elif operation == "get_content_from_model_response":
        response = ModelResponse(**cast(dict[str, object], input_value["response"]))
        result = common_utils.get_content_from_model_response(response)  # pyright: ignore[reportUnknownMemberType]
    else:
        raise ValueError(f"unsupported operation {operation}")
    return cast(Json, result)
