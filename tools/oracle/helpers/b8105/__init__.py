import litellm

from .operations import create_plan, create_reply, headers_and_url, requests
from .views import Input, Json

__all__ = ["litellm", "run"]


def run(input_value: Input) -> Json:
    if input_value["op"] == "headers_and_url":
        return headers_and_url(input_value)
    if input_value["op"] == "create_plan":
        return create_plan(input_value)
    if input_value["op"] == "create_reply":
        return create_reply(input_value)
    return requests(input_value)
