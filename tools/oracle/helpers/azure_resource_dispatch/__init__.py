import litellm
from pydantic import JsonValue

from .assistants import one_step as assistant_step
from .container_video_vector import container_video_vector
from .contracts import (
    ASSISTANT_CASE,
    CONTAINER_CASE,
    FILES_CASE,
    FINE_TUNING_CASE,
    PASSTHROUGH_CASE,
    REALTIME_CASE,
    RESPONSES_CASE,
    Input,
    stepped,
)
from .files_batches import one_step as file_step
from .fine_tuning import job_answer
from .passthrough import passthrough
from .realtime import realtime
from .responses_search import one_step as responses_step

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True


def run(input_value: Input) -> JsonValue:
    case = input_value["case"]
    answer: JsonValue
    if case == "files_batches":
        answer = stepped(FILES_CASE.validate_python(input_value)["steps"], file_step)
    elif case == "fine_tuning":
        answer = stepped(FINE_TUNING_CASE.validate_python(input_value)["steps"], job_answer)
    elif case == "assistant_thread_run":
        answer = stepped(ASSISTANT_CASE.validate_python(input_value)["steps"], assistant_step)
    elif case == "responses_search":
        answer = stepped(RESPONSES_CASE.validate_python(input_value)["steps"], responses_step)
    elif case == "realtime":
        answer = realtime(REALTIME_CASE.validate_python(input_value))
    elif case == "container_video_vector":
        answer = container_video_vector(CONTAINER_CASE.validate_python(input_value))
    elif case == "passthrough":
        answer = passthrough(PASSTHROUGH_CASE.validate_python(input_value))
    else:
        raise ValueError(f"unknown azure resource dispatch case {case}")
    return answer
