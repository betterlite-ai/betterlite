from pydantic import JsonValue

from .contracts import Input, JsonObject
from .replies import reply_value
from .values import (
    auth_value,
    model_value,
    outcomes,
    supported_value,
    transform_value,
    url_value,
)


def run(values: Input) -> JsonValue:
    answer: JsonObject = {}
    if "models" in values:
        answer["models"] = [model_value(model) for model in values["models"]]
    if "supported" in values:
        answer["supported"] = [supported_value(model) for model in values["supported"]]
    if "auths" in values:
        answer["auths"] = [auth_value(step) for step in values["auths"]]
    if "urls" in values:
        answer["urls"] = outcomes(values["urls"], url_value)
    if "transforms" in values:
        answer["transforms"] = outcomes(values["transforms"], transform_value)
    if "replies" in values:
        answer["replies"] = [reply_value(step) for step in values["replies"]]
    return answer
