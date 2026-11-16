from .answers import (
    annotation_answer,
    extraction_answer,
    reply_answer,
    request_answer,
    stream_answer,
)
from .types import Input, JsonObject


def run(argument: Input) -> JsonObject:
    answer: JsonObject = {}
    if "extractions" in argument:
        answer["extractions"] = [extraction_answer(call) for call in argument["extractions"]]
    if "annotations" in argument:
        answer["annotations"] = [annotation_answer(call) for call in argument["annotations"]]
    if "replies" in argument:
        answer["replies"] = [reply_answer(call) for call in argument["replies"]]
    if "streams" in argument:
        answer["streams"] = [stream_answer(call) for call in argument["streams"]]
    if "requests" in argument:
        answer["requests"] = [request_answer(call) for call in argument["requests"]]
    return answer
