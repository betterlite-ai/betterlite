from pydantic import JsonValue

from .inputs import Input, ReplyInput
from .reports import (
    header_report,
    pair_report,
    reply_report,
    request_report,
    settings_report,
)


def reply_payload(input_value: Input) -> ReplyInput:
    if "citations" in input_value:
        return input_value["citations"]
    if "reply" in input_value:
        return input_value["reply"]
    if "error" in input_value:
        return input_value["error"]
    if "price" in input_value:
        return input_value["price"]
    raise ValueError("expected an azure_search reply variant")


def run(input_value: Input) -> JsonValue:
    if "request" in input_value:
        report = request_report(input_value["request"])
    elif "headers" in input_value:
        report = header_report(input_value["headers"])
    elif "settings" in input_value:
        report = settings_report(input_value["settings"])
    elif "max_results" in input_value:
        report = pair_report(input_value["max_results"])
    elif "cap" in input_value:
        report = pair_report(input_value["cap"])
    else:
        report = reply_report(reply_payload(input_value))
    return report
