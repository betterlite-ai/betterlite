from litellm.llms.edenai.common_utils import reported_cost
from pydantic import JsonValue

from .calls import (
    catalog_value,
    chat_params_value,
    credentials_value,
    embedding_plan_value,
    messages_environment_value,
    prepare_value,
)
from .configs import CHAT
from .replies import (
    chat_reply_value,
    chunk_value,
    embedding_reply_value,
    messages_reply_value,
    responses_reply_value,
)
from .wire import Input


def cost_steps(input_value: Input) -> JsonValue | None:
    if input_value["costs"]:
        return {"steps": [reported_cost(payload) for payload in input_value["costs"]]}
    if input_value["cost_bodies"]:
        return {"steps": [reported_cost(body.encode()) for body in input_value["cost_bodies"]]}
    if input_value["cost_headers"]:
        return {"steps": [reported_cost(headers) for headers in input_value["cost_headers"]]}
    return None


def call_steps(input_value: Input) -> JsonValue | None:
    if input_value["credentials"]:
        return {"steps": [credentials_value(case) for case in input_value["credentials"]]}
    if input_value["chat_params"]:
        return {"steps": [chat_params_value(case) for case in input_value["chat_params"]]}
    if input_value["chat_requests"]:
        steps = [
            CHAT.transform_request(case["model"], [], case["optional_params"], {}, {})
            for case in input_value["chat_requests"]
        ]
        return {"steps": steps}
    if input_value["chat_replies"]:
        return {"steps": [chat_reply_value(body) for body in input_value["chat_replies"]]}
    if input_value["chunks"]:
        return {"steps": [chunk_value(chunk) for chunk in input_value["chunks"]]}
    return None


def plan_steps(input_value: Input) -> JsonValue | None:
    if input_value["models"]:
        return {"steps": [catalog_value(case) for case in input_value["models"]]}
    if input_value["embedding_plans"]:
        steps = [embedding_plan_value(case) for case in input_value["embedding_plans"]]
        return {"steps": steps}
    if input_value["embedding_replies"]:
        steps = [embedding_reply_value(body) for body in input_value["embedding_replies"]]
        return {"steps": steps}
    if input_value["responses_prepare"]:
        steps = [prepare_value(case) for case in input_value["responses_prepare"]]
        return {"steps": steps}
    if input_value["responses_replies"]:
        return {"steps": [responses_reply_value(body) for body in input_value["responses_replies"]]}
    return None


def reply_steps(input_value: Input) -> JsonValue | None:
    if input_value["messages_environments"]:
        steps = [messages_environment_value(case) for case in input_value["messages_environments"]]
        return {"steps": steps}
    if input_value["messages_replies"]:
        return {"steps": [messages_reply_value(body) for body in input_value["messages_replies"]]}
    return None


def run(input_value: Input) -> JsonValue:
    for group in (cost_steps, call_steps, plan_steps, reply_steps):
        answer = group(input_value)
        if answer is not None:
            return answer
    raise ValueError("expected one b6100 variant")
