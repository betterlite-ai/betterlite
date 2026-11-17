from .answers import dispatch_answer, run_answer
from .types import Input, JsonObject


def run(argument: Input) -> JsonObject:
    answer: JsonObject = {}
    if "runs" in argument:
        answer["runs"] = [run_answer(call) for call in argument["runs"]]
    if "dispatches" in argument:
        answer["dispatches"] = [
            dispatch_answer(call) for call in argument["dispatches"]
        ]
    return answer
