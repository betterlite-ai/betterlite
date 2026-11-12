from typing import cast

from litellm.types.responses.main import OutputCodeInterpreterCall
from openai.types.responses.response_function_web_search import ResponseFunctionWebSearch

from anthropic_stream_state.records import Decoder, Dumpable, Json


def block_view(block: Json) -> Json:
    fields = cast(dict[str, Json], block)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/handler.py#L670
    delta = cast(dict[str, Json], fields["delta"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/anthropic.py#L484
    keys = ("type", "text", "partial_json", "citation", "thinking", "signature", "content")
    return {
        "type": fields.get("type"),
        "index": fields.get("index"),
        "delta": {key: delta.get(key) for key in keys},
    }


def start_view(start: Json) -> Json:
    fields = cast(dict[str, Json], start)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/handler.py#L752
    block = cast(dict[str, Json], fields.get("content_block"))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/anthropic.py#L530
    keys = ("type", "text", "id", "name", "input", "data", "content", "caller")
    return {"index": fields.get("index"), "content_block": {key: block.get(key) for key in keys}}


def tracked_calls(calls: dict[str, Json]) -> dict[str, Dumpable]:
    tracked: dict[str, Dumpable] = {}
    for key, value in calls.items():
        tracked[key] = ResponseFunctionWebSearch.model_validate(value)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/responses/main.py#L61
    return tracked


def calls_view(decoder: Decoder) -> dict[str, Json]:
    return {
        key: call.model_dump()
        for key, call in decoder._web_search_call_snapshot().items()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/handler.py#L765
    }


def results_view(decoder: Decoder) -> list[Json]:
    calls: list[OutputCodeInterpreterCall] = decoder._build_interpreter_results()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/handler.py#L778
    return [cast(Json, call.model_dump()) for call in calls]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/responses/main.py#L90


def state_view(decoder: Decoder) -> Json:
    return {
        "content_blocks": [block_view(block) for block in decoder.content_blocks],
        "tool_index": decoder.tool_index,
        "json_mode": decoder.json_mode,
        "speed": decoder.speed,
        "tool_name_reverse_map": decoder.tool_name_reverse_map,
        "response_id": decoder.response_id,
        "served_model": decoder.served_model,
        "is_response_format_tool": decoder.is_response_format_tool,
        "converted_response_format_tool": decoder.converted_response_format_tool,
        "chunk_type": decoder.chunk_type,
        "current_content_block_type": decoder.current_content_block_type,
        "web_search_results": decoder.web_search_results,
        "web_search_calls": calls_view(decoder),
        "compaction_blocks": decoder.compaction_blocks,
        "reasoning_content_chunks": decoder.reasoning_content_chunks,
        "server_tool_inputs": decoder._server_tool_inputs,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/handler.py#L599
        "tool_results": decoder.tool_results,
        "current_server_tool_id": decoder._current_server_tool_id,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/handler.py#L601
        "container_id": decoder._container_id,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/handler.py#L602
        "accumulated_json": decoder.accumulated_json,
    }
