from enum import Enum


class ResponsesAPIStreamEvents(str, Enum):
    RESPONSE_CREATED = "response.created"
    RESPONSE_IN_PROGRESS = "response.in_progress"
    RESPONSE_COMPLETED = "response.completed"
    RESPONSE_FAILED = "response.failed"
    RESPONSE_INCOMPLETE = "response.incomplete"

    RESPONSE_PART_ADDED = "response.reasoning_summary_part.added"
    REASONING_SUMMARY_TEXT_DELTA = "response.reasoning_summary_text.delta"
    REASONING_SUMMARY_TEXT_DONE = "response.reasoning_summary_text.done"
    REASONING_SUMMARY_PART_DONE = "response.reasoning_summary_part.done"

    OUTPUT_ITEM_ADDED = "response.output_item.added"
    OUTPUT_ITEM_DONE = "response.output_item.done"

    CONTENT_PART_ADDED = "response.content_part.added"
    CONTENT_PART_DONE = "response.content_part.done"

    OUTPUT_TEXT_DELTA = "response.output_text.delta"
    OUTPUT_TEXT_ANNOTATION_ADDED = "response.output_text.annotation.added"
    OUTPUT_TEXT_DONE = "response.output_text.done"

    REFUSAL_DELTA = "response.refusal.delta"
    REFUSAL_DONE = "response.refusal.done"

    FUNCTION_CALL_ARGUMENTS_DELTA = "response.function_call_arguments.delta"
    FUNCTION_CALL_ARGUMENTS_DONE = "response.function_call_arguments.done"

    CUSTOM_TOOL_CALL_INPUT_DELTA = "response.custom_tool_call_input.delta"
    CUSTOM_TOOL_CALL_INPUT_DONE = "response.custom_tool_call_input.done"

    FILE_SEARCH_CALL_IN_PROGRESS = "response.file_search_call.in_progress"
    FILE_SEARCH_CALL_SEARCHING = "response.file_search_call.searching"
    FILE_SEARCH_CALL_COMPLETED = "response.file_search_call.completed"

    WEB_SEARCH_CALL_IN_PROGRESS = "response.web_search_call.in_progress"
    WEB_SEARCH_CALL_SEARCHING = "response.web_search_call.searching"
    WEB_SEARCH_CALL_COMPLETED = "response.web_search_call.completed"

    MCP_LIST_TOOLS_IN_PROGRESS = "response.mcp_list_tools.in_progress"
    MCP_LIST_TOOLS_COMPLETED = "response.mcp_list_tools.completed"
    MCP_LIST_TOOLS_FAILED = "response.mcp_list_tools.failed"
    MCP_CALL_IN_PROGRESS = "response.mcp_call.in_progress"
    MCP_CALL_ARGUMENTS_DELTA = "response.mcp_call_arguments.delta"
    MCP_CALL_ARGUMENTS_DONE = "response.mcp_call_arguments.done"
    MCP_CALL_COMPLETED = "response.mcp_call.completed"
    MCP_CALL_FAILED = "response.mcp_call.failed"

    IMAGE_GENERATION_PARTIAL_IMAGE = "image_generation.partial_image"

    SHELL_CALL_IN_PROGRESS = "response.shell_call.in_progress"
    SHELL_CALL_COMPLETED = "response.shell_call.completed"
    SHELL_CALL_OUTPUT = "response.shell_call_output.done"

    ERROR = "error"
