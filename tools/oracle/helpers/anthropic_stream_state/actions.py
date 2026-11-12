from typing import cast

from litellm.types.utils import Usage

from anthropic_stream_state.records import Decoder, Json, StateFields
from anthropic_stream_state.views import calls_view, state_view, tracked_calls


def delta_answer(decoder: Decoder, chunk: dict[str, Json]) -> Json:
    text, tool_use, blocks, providers, reasoning = decoder._content_block_delta_helper(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/handler.py#L648
        chunk=chunk
    )
    return {
        "text": text,
        "tool_use": tool_use,
        "thinking_blocks": blocks,
        "provider_specific_fields": providers,
        "reasoning_content": reasoning,
    }


def redacted_answer(decoder: Decoder, chunk: dict[str, Json]) -> Json:
    start = decoder.get_content_block_start(chunk=chunk)
    providers: dict[str, Json] = {}
    blocks, providers = decoder._handle_redacted_thinking(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/handler.py#L734
        content_block_start=start, provider_specific_fields=providers
    )
    return {"thinking_blocks": blocks, "provider_specific_fields": providers}


def usage_answer(decoder: Decoder, usage: dict[str, Json]) -> Json:
    calculated: Usage = decoder._handle_usage(anthropic_usage_chunk=usage)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/handler.py#L635
    return {
        "usage": cast(Json, calculated.model_dump()),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1857
        "cumulative_usage": dict(decoder._cumulative_usage),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/handler.py#L560
    }


def message_delta_answer(decoder: Decoder, chunk: dict[str, Json]) -> Json:
    finish_reason, usage, container = decoder._handle_message_delta(chunk=chunk)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/handler.py#L1096
    answered = None if usage is None else cast(Json, usage.model_dump())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1857
    return {
        "projection": {
            "finish_reason": finish_reason,
            "usage": answered,
            "container": container,
        }
    }


def setup_state(decoder: Decoder, fields: StateFields) -> Json:
    if "content_blocks" in fields:
        decoder.content_blocks = fields["content_blocks"]
    if "web_search_calls" in fields:
        decoder._web_search_calls = tracked_calls(fields["web_search_calls"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/handler.py#L589
    if "server_tool_inputs" in fields:
        decoder._server_tool_inputs = fields["server_tool_inputs"]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/handler.py#L599
    if "tool_results" in fields:
        decoder.tool_results = fields["tool_results"]
    if "container_id" in fields:
        decoder._container_id = fields["container_id"]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/handler.py#L602
    if "current_content_block_type" in fields:
        decoder.current_content_block_type = fields["current_content_block_type"]
    if "tool_index" in fields:
        decoder.tool_index = fields["tool_index"]
    if "converted_response_format_tool" in fields:
        decoder.converted_response_format_tool = fields["converted_response_format_tool"]
    if "accumulated_json" in fields:
        decoder.accumulated_json = fields["accumulated_json"]
    return state_view(decoder)


def complete_web_search_answer(decoder: Decoder, result: dict[str, Json]) -> Json:
    decoder._complete_web_search_call(result=result)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/handler.py#L768
    return {"calls": calls_view(decoder)}
