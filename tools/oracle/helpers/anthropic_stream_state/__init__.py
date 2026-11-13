from typing import cast
from uuid import UUID

import litellm.types.utils
from litellm.llms.anthropic.chat.handler import ModelResponseIterator

from anthropic_stream_state.actions import (
    complete_web_search_answer,
    delta_answer,
    message_delta_answer,
    redacted_answer,
    setup_state,
    usage_answer,
)
from anthropic_stream_state.records import (
    BlockStartStep,
    CheckEmptyStep,
    CodeResultsStep,
    CompleteWebSearchStep,
    Decoder,
    DeltaStep,
    Input,
    Json,
    MessageDeltaStep,
    RedactedStep,
    StateStep,
    Step,
    UsageStep,
    WebSnapshotStep,
)
from anthropic_stream_state.views import calls_view, results_view, start_view, state_view


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
        input_value["sync_stream"],
        json_mode=input_value["json_mode"],
        speed=input_value["speed"],
        tool_name_reverse_map=input_value["tool_name_reverse_map"],
    )
    return cast(Decoder, created)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/handler.py#L546


def _state_answer(decoder: Decoder, step: CheckEmptyStep | StateStep) -> Json:
    match step["function"]:
        case "check_empty":
            return {"boolean": decoder.check_empty_tool_call_args()}
        case "state":
            return {"state": state_view(decoder)}
    raise ValueError("unknown stream state step")


def _block_answer(decoder: Decoder, step: DeltaStep | BlockStartStep | RedactedStep) -> Json:
    match step["function"]:
        case "delta":
            return {"delta": delta_answer(decoder, step["chunk"])}
        case "block_start":
            return {"start": start_view(decoder.get_content_block_start(chunk=step["chunk"]))}
        case "redacted":
            return redacted_answer(decoder, step["chunk"])
    raise ValueError("unknown stream state step")


def _usage_answer(
    decoder: Decoder,
    step: UsageStep | MessageDeltaStep | WebSnapshotStep | CompleteWebSearchStep | CodeResultsStep,
) -> Json:
    match step["function"]:
        case "usage":
            return usage_answer(decoder, step["usage"])
        case "message_delta":
            return message_delta_answer(decoder, step["chunk"])
        case "web_snapshot":
            return {"calls": calls_view(decoder)}
        case "complete_web_search":
            return complete_web_search_answer(decoder, step["result"])
        case "code_results":
            return {"results": results_view(decoder)}
    raise ValueError("unknown stream state step")


def _run_step(decoder: Decoder, step: Step) -> Json:
    try:
        match step["function"]:
            case "setup":
                return {"state": setup_state(decoder, step["fields"])}
            case "check_empty" | "state":
                return _state_answer(decoder, step)
            case "delta" | "block_start" | "redacted":
                return _block_answer(decoder, step)
            case _:
                return _usage_answer(decoder, step)
    except Exception:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/chat/handler.py#L811
        return {"failed": True}


def run(input_value: Input) -> Json:
    decoder = _iterator(input_value)
    steps = [_run_step(decoder, step) for step in input_value["steps"]]
    return {"steps": steps}
