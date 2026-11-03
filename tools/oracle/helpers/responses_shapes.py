from typing import Literal, TypeAlias, cast

from litellm.types.responses.main import (
    CustomToolCallOutputItem,
    DecodedResponseId,
    DeleteResponseResult,
    GenericResponseOutputItem,
    OutputCodeInterpreterCall,
    OutputFunctionToolCall,
    OutputImageGenerationCall,
    OutputText,
    build_code_interpreter_log_outputs,
    build_web_search_call,
)
from litellm.types.responses.streaming_websocket import ResponsesWebSocketRequestDefaults
from pydantic import TypeAdapter

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(fields: dict[str, Json]) -> Json:
    operation = fields["operation"]
    if operation == "web_search_sources":
        result = build_web_search_call(
            tool_id=cast(str, fields["tool_id"]),
            tool_input=fields["tool_input"],
            result=fields["result"],
            status=cast(
                Literal["in_progress", "searching", "completed", "failed"] | None,
                fields.get("status"),
            ),
        )
        return cast(Json, result.model_dump(exclude_none=False))
    if operation == "interpreter_logs":
        result = build_code_interpreter_log_outputs(fields["content"])
        return None if result is None else cast(Json, [item.model_dump() for item in result])
    if operation == "nullable_records":
        record = DeleteResponseResult.model_validate(fields["record"])
        # Construct pinned records too, so missing required nullable fields remain validated.
        output_text = OutputText.model_validate(fields["output_text"])
        function_call = OutputFunctionToolCall.model_validate(fields["function_call"])
        image_call = OutputImageGenerationCall.model_validate(fields["image_call"])
        interpreter_call = OutputCodeInterpreterCall.model_validate(fields["interpreter_call"])
        return cast(
            Json,
            {
                "record": record.model_dump(exclude_none=False),
                "output_text": output_text.model_dump(exclude_none=False),
                "function_call": function_call.model_dump(exclude_none=False),
                "image_call": image_call.model_dump(exclude_none=False),
                "interpreter_call": interpreter_call.model_dump(exclude_none=False),
                "custom_call": CustomToolCallOutputItem.model_validate(
                    fields["custom_call"]
                ).model_dump(),
                "generic_item": GenericResponseOutputItem.model_validate(
                    fields["generic_item"]
                ).model_dump(),
                "decoded_id": TypeAdapter(DecodedResponseId).validate_python(fields["decoded_id"]),
            },
        )
    if operation == "websocket_defaults":
        defaults = ResponsesWebSocketRequestDefaults(
            fill_missing=cast(dict[str, object], fields["fill_missing"]),
            overrides=cast(dict[str, object], fields["overrides"]),
        )
        return cast(Json, defaults.merged_into(cast(dict[str, object], fields["request"])))
    raise ValueError(f"unsupported responses_shapes operation: {operation}")
