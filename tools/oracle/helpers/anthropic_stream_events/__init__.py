from copy import deepcopy
from typing import cast
from uuid import UUID

import litellm.types.utils
from litellm.llms.anthropic.chat.handler import ModelResponseIterator
from litellm.llms.anthropic.common_utils import AnthropicError

from anthropic_stream_events.records import Decoder, Dumpable, Input, Json, Step


class FixedUuid:
    def __init__(self, values: list[UUID]) -> None:
        self.values = values

    def __call__(self) -> UUID:
        return self.values.pop(0)


def _iterator(input_value: Input) -> Decoder:
    fixed = FixedUuid([UUID(value) for value in input_value["ids"]])
    litellm.types.utils.uuid.uuid4 = fixed  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L122
    created = ModelResponseIterator(
        None,
        sync_stream=False,
        json_mode=input_value["json_mode"],
        speed=None,
        tool_name_reverse_map=input_value["tool_name_reverse_map"],
    )
    return cast(Decoder, created)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/handler.py#L546


def _chunk_view(chunk: Dumpable) -> Json:
    dumped = cast(dict[str, Json], chunk.model_dump())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2096
    choice = cast(dict[str, Json], cast(list[Json], dumped["choices"])[0])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2006
    delta = cast(dict[str, Json], choice["delta"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L1507
    delta_view: dict[str, Json] = {
        "content": delta["content"],
        "provider_specific_fields": delta.get("provider_specific_fields"),
    }
    for key in ("tool_calls", "thinking_blocks", "reasoning_content"):
        value = delta.get(key)
        if value is not None:
            delta_view[key] = value
    view: dict[str, Json] = {
        "id": dumped["id"],
        "model": dumped.get("model"),
        "choices": [
            {
                "index": choice["index"],
                "finish_reason": choice.get("finish_reason"),
                "delta": delta_view,
            }
        ],
        "usage": dumped.get("usage"),
    }
    return deepcopy(view)


def _state_view(iterator: Decoder) -> Json:
    view: dict[str, Json] = {
        "current_content_block_type": iterator.current_content_block_type,
        "current_server_tool_id": iterator._current_server_tool_id,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/handler.py#L601
        "is_response_format_tool": iterator.is_response_format_tool,
        "converted_response_format_tool": iterator.converted_response_format_tool,
        "tool_index": iterator.tool_index,
        "served_model": iterator.served_model,
        "container_id": iterator._container_id,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/handler.py#L602
        "server_tool_inputs": iterator._server_tool_inputs,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/handler.py#L599
    }
    return deepcopy(view)


def _run_step(iterator: Decoder, step: Step) -> Json:
    try:
        match step["function"]:
            case "chunk":
                return {"chunk": _chunk_view(iterator.chunk_parser(chunk=step["chunk"]))}
            case "accumulated":
                chunk = iterator._handle_accumulated_json_chunk(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/handler.py#L1118
                    data_str=step["fragment"], is_final=step["is_final"]
                )
                return {"accumulated": None if chunk is None else _chunk_view(chunk)}
            case "state":
                return {"state": _state_view(iterator)}
    except AnthropicError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/common_utils.py#L309
        return {"error": {"status": error.status_code}}
    except Exception:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/handler.py#L811
        return {"failed": True}


def run(input_value: Input) -> Json:
    iterator = _iterator(input_value)
    return {"steps": [_run_step(iterator, step) for step in input_value["steps"]]}
