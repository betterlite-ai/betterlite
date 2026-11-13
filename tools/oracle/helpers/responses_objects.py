from typing import TypeAlias, cast

from litellm.types.llms.openai import (
    ResponseAPIUsage,
    ResponsesAPIResponse,
    ResponsesToolUsage,
)
from pydantic import JsonValue, RootModel

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(fields: dict[str, Json]) -> Json:
    operation = fields["operation"]
    if operation == "usage_cost":
        usage = ResponseAPIUsage.model_validate(fields["usage"])
        return cast(Json, usage.model_dump(exclude_none=False))
    if operation == "response_defaults":
        response = ResponsesAPIResponse.model_validate(fields["response"])
        return cast(
            Json,
            {
                "response": response.model_dump(exclude_none=False),
                "output_text": response.output_text,
            },
        )
    if operation in ("reasoning_nulls", "reasoning_model"):
        model_input = dict(cast(dict[str, object], fields["response"]))
        if operation == "reasoning_model":
            model_input["reasoning"] = RootModel[dict[str, JsonValue]].model_validate(
                model_input["reasoning"]
            )
            model_input["usage"] = ResponseAPIUsage.model_validate(model_input["usage"])
        response = ResponsesAPIResponse.model_validate(model_input)
        return cast(Json, response.model_dump(exclude_none=False))
    if operation == "output_text_join":
        response = ResponsesAPIResponse.model_validate(fields["response"])
        return cast(Json, response.output_text)
    if operation == "tool_usage_count":
        usage = ResponsesToolUsage.model_validate(fields["tool_usage"])
        return cast(Json, usage.model_dump(exclude_none=False))
    raise ValueError(f"unsupported responses_objects operation: {operation}")
