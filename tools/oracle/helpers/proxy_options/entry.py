from typing import Literal, TypedDict

from pydantic import JsonValue

from .chat import chat_cases
from .image import image_cases
from .models import models_cases
from .responses import responses_cases
from .route import route_cases


class Input(TypedDict):
    operation: Literal["route", "chat_request", "responses_url", "image_request", "models"]
    params: JsonValue


def run(input_value: Input) -> JsonValue:
    operation = input_value["operation"]
    params = input_value["params"]
    if operation == "route":
        return route_cases(params)
    if operation == "chat_request":
        return chat_cases(params)
    if operation == "responses_url":
        return responses_cases(params)
    if operation == "image_request":
        return image_cases(params)
    return models_cases(params)
