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
    chunk_value,
    messages_reply_value,
    responses_reply_value,
    transformed_reply,
)
from .wire import Input


def cost_steps(input_value: Input) -> JsonValue | None:
    if input_value["costs"]:
        return {
            "steps": list[JsonValue](
                reported_cost(payload) for payload in input_value["costs"]
            )
        }
    if input_value["cost_bodies"]:
        return {
            "steps": list[JsonValue](
                reported_cost(body.encode()) for body in input_value["cost_bodies"]
            )
        }
    if input_value["cost_headers"]:
        return {
            "steps": list[JsonValue](
                reported_cost(headers) for headers in input_value["cost_headers"]
            )
        }
    return None


def call_steps(input_value: Input) -> JsonValue | None:
    if input_value["credentials"]:
        steps = list[JsonValue](credentials_value(case) for case in input_value["credentials"])
        return {"steps": steps}
    if input_value["chat_params"]:
        steps = list[JsonValue](chat_params_value(case) for case in input_value["chat_params"])
        return {"steps": steps}
    if input_value["chat_requests"]:
        steps = list[JsonValue](
            CHAT.transform_request(case["model"], [], case["optional_params"], {}, {})
            for case in input_value["chat_requests"]
        )
        return {"steps": steps}
    if input_value["chat_replies"]:
        steps = list[JsonValue](
            transformed_reply(body, "chat") for body in input_value["chat_replies"]
        )
        return {"steps": steps}
    if input_value["chunks"]:
        steps = list[JsonValue](chunk_value(chunk) for chunk in input_value["chunks"])
        return {"steps": steps}
    return None


def plan_steps(input_value: Input) -> JsonValue | None:
    steps: list[JsonValue] | None = None
    if input_value["models"]:
        steps = list[JsonValue](catalog_value(case) for case in input_value["models"])
    elif input_value["embedding_plans"]:
        steps = list[JsonValue](
            embedding_plan_value(case) for case in input_value["embedding_plans"]
        )
    elif input_value["embedding_replies"]:
        steps = list[JsonValue](
            transformed_reply(body, "embedding") for body in input_value["embedding_replies"]
        )
    elif input_value["responses_prepare"]:
        steps = list[JsonValue](
            prepare_value(case) for case in input_value["responses_prepare"]
        )
    elif input_value["responses_replies"]:
        steps = list[JsonValue](
            responses_reply_value(body) for body in input_value["responses_replies"]
        )
    if steps is None:
        return None
    return {"steps": steps}


def reply_steps(input_value: Input) -> JsonValue | None:
    if input_value["messages_environments"]:
        steps = list[JsonValue](
            messages_environment_value(case) for case in input_value["messages_environments"]
        )
        return {"steps": steps}
    if input_value["messages_replies"]:
        steps = list[JsonValue](
            messages_reply_value(body) for body in input_value["messages_replies"]
        )
        return {"steps": steps}
    return None


def run(input_value: Input) -> JsonValue:
    for group in (cost_steps, call_steps, plan_steps, reply_steps):
        answer = group(input_value)
        if answer is not None:
            return answer
    raise ValueError("expected one b6100 variant")
