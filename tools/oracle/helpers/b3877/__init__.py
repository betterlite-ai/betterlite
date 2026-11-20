from pydantic import JsonValue

from .calls import Input
from .payload import build_payload
from .probe import ProbeLogger, label_values


def run(input_value: Input) -> JsonValue:
    answer: JsonValue = None
    match input_value["function"]:
        case "request_and_spend":
            logger = ProbeLogger(input_value["supported_labels"], [])
            logger.request_spend(label_values(input_value["values"]), input_value["response_cost"])
            answer = {"events": logger.entries()}
        case "zero_cost":
            logger = ProbeLogger(input_value["supported_labels"], [])
            logger.zero_cost(label_values(input_value["values"]))
            answer = {"events": logger.entries()}
        case "token_metrics":
            logger = ProbeLogger(input_value["supported_labels"], [])
            logger.token_metrics(build_payload(input_value), label_values(input_value["values"]))
            answer = {"events": logger.entries()}
        case "token_detail":
            logger = ProbeLogger(input_value["supported_labels"], [])
            logger.token_detail(build_payload(input_value), label_values(input_value["values"]))
            answer = {"events": logger.entries()}
        case "media":
            logger = ProbeLogger(input_value["supported_labels"], [])
            logger.media(build_payload(input_value), label_values(input_value["values"]))
            answer = {"events": logger.entries()}
        case "cache":
            logger = ProbeLogger(input_value["supported_labels"], [])
            logger.cache(build_payload(input_value), label_values(input_value["values"]))
            answer = {"events": logger.entries()}
        case "provider_cache_tokens":
            read, creation = ProbeLogger.provider_cache(input_value["provider_usage"])
            answer = {"read": read, "creation": creation}
        case "cache_write_tokens":
            answer = {"cache_write_tokens": ProbeLogger.cache_write(input_value["prompt_details"])}
    return answer
