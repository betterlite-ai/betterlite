from .operations import flow_value, replies_value, requests_value
from .shapes import Input, Json


def run(input_value: Input) -> Json:
    if input_value["op"] == "flow":
        return flow_value(input_value)
    if input_value["op"] == "request":
        return requests_value(input_value)
    return replies_value(input_value)
