from typing import Literal, NotRequired, TypeAlias, TypedDict

from litellm.completion_extras.litellm_responses_transformation.transformation import (
    LiteLLMResponsesTransformationHandler as ResponsesHandler,
)
from litellm.responses.sse_output_recovery import parse_sse_json_chunk
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

# LiteLLM hands back plain dicts and lists; this checks those values as JSON instead of casting them
JSON: TypeAdapter[Json] = TypeAdapter(Json)

Action: TypeAlias = Literal["recovery_line_filter", "recover_recorded_output"]


class Input(TypedDict):
    action: Action
    lines: NotRequired[list[str]]
    recordings: NotRequired[list[str]]


def run(input_value: Input) -> Json:
    action = input_value["action"]
    if action == "recovery_line_filter":
        lines = input_value.get("lines")
        if lines is None:
            raise ValueError("the lines field is required")
        return JSON.validate_python(
            [parse_sse_json_chunk(line) for line in lines]  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/sse_output_recovery.py#L21-L41
        )
    recordings = input_value.get("recordings")
    if recordings is None:
        raise ValueError("the recordings field is required")
    recovered = [
        ResponsesHandler._recover_output_items_from_raw_sse(recording)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/completion_extras/litellm_responses_transformation/transformation.py#L836
        for recording in recordings
    ]
    return JSON.validate_python(recovered)
