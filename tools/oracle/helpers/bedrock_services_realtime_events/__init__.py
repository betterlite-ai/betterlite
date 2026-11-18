import fastuuid
from litellm.litellm_core_utils.litellm_logging import Logging
from pydantic import JsonValue

from bedrock_services_realtime_session import FixedFast, make_logging

from .handlers import (
    ack_step,
    client_messages_step,
    dispatch_step,
    parse_client_step,
    pending_step,
    provider_step,
    should_log_step,
)
from .models import CONFIG_TYPE, Input, JsonObject, RealtimeConfig, Step


def run_step(config: RealtimeConfig, step: Step, logging_obj: Logging) -> JsonObject:
    result: JsonObject
    if step["method"] == "dispatch":
        result = dispatch_step(config, step, logging_obj)
    elif step["method"] == "client_messages":
        result = client_messages_step(step)
    elif step["method"] == "parse_client_message":
        result = parse_client_step(step)
    elif step["method"] == "pending":
        result = pending_step(step)
    elif step["method"] == "provider_failure":
        result = provider_step(step)
    elif step["method"] == "ack":
        result = ack_step(config, step)
    else:
        result = should_log_step(step)
    return result


def run(input_value: Input) -> JsonValue:
    fastuuid.uuid4 = FixedFast(input_value["ids"])
    config = CONFIG_TYPE()
    logging_obj = make_logging("dispatch")
    steps: list[JsonValue] = [run_step(config, step, logging_obj) for step in input_value["steps"]]
    return {"steps": steps}
