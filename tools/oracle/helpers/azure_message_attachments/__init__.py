import litellm
from pydantic import JsonValue

from azure_message_attachments.sdk import (
    ADD,
    GET_MESSAGES,
    MAP,
    MESSAGE,
    PAGE,
    SUPPORTED,
    Input,
    MapStep,
    Params,
    Step,
    kind_of,
    message_data,
    params_of,
    report,
    reported,
    session,
    settle,
    supplied_client,
    values_of,
)


def append_step(step: Step) -> JsonValue:
    values = values_of(step)
    params = params_of(values)
    params["message_data"] = message_data(step)
    params["a_add_message"] = values.flag
    with session(step, MESSAGE) as wire, reported(wire):
        wire.result = settle(ADD(**params, client=supplied_client(step, wire)))
    return report(wire, kind_of(step, values.flag))


def list_step(step: Step) -> JsonValue:
    values = values_of(step)
    params = params_of(values)
    params["aget_messages"] = values.flag
    with session(step, PAGE) as wire, reported(wire):
        wire.result = settle(GET_MESSAGES(**params, client=supplied_client(step, wire)))
    return report(wire, kind_of(step, values.flag))


def map_step(step: MapStep) -> JsonValue:
    non_default: Params = {}
    if "non_default" in step:
        non_default = dict(step["non_default"])
    optional: Params = {}
    if "optional" in step:
        optional = dict(step["optional"])
    drop = False
    if "drop_params" in step:
        drop = step["drop_params"]
    supported = SUPPORTED()
    saved = litellm.drop_params
    litellm.drop_params = drop
    try:
        mapped = MAP(non_default, optional)
    finally:
        litellm.drop_params = saved
    values: dict[str, JsonValue] = {"supported": supported, "optional": mapped}
    return values


def run(argument: Input) -> JsonValue:
    steps: list[JsonValue] = []
    if "append_message" in argument:
        steps = [append_step(step) for step in argument["append_message"]]
    elif "list_messages" in argument:
        steps = [list_step(step) for step in argument["list_messages"]]
    elif "map_create_message_params" in argument:
        steps = [map_step(step) for step in argument["map_create_message_params"]]
    else:
        raise ValueError("expected one azure message attachments variant")
    reports: dict[str, JsonValue] = {"steps": steps}
    return reports
