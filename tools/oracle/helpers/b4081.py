from decimal import Decimal
from types import SimpleNamespace
from typing import Final, Literal, TypedDict

import litellm
from litellm.integrations import shadow_eval_logger
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

JSON: Final = TypeAdapter(JsonValue)
chat_text, finish_reason, empty_error, responses_text, call_cost, judge_shape, routing_decision, routed_tier, classifier_cost = (shadow_eval_logger._chat_final_text, shadow_eval_logger._chat_finish_reason, shadow_eval_logger._shadow_empty_reply_error, shadow_eval_logger._responses_final_text, shadow_eval_logger._call_cost, shadow_eval_logger._judge_reply_shape, shadow_eval_logger._routing_decision, shadow_eval_logger._routed_tier, shadow_eval_logger._decision_classifier_cost)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/shadow_eval_logger.py#L171-L731


class Input(TypedDict):
    operation: Literal["chat_text", "responses_text", "call_cost", "judge_shape", "routing"]
    response: dict[str, JsonValue]
    metadata: dict[str, JsonValue]
    routed_model: str


def cost_response(payload: dict[str, JsonValue]) -> dict[str, JsonValue] | SimpleNamespace:
    # the stamped cost lives on an attribute, so the payload needs an object to carry it
    if "_hidden_params" in payload:
        return SimpleNamespace(**payload)
    return payload


def run(input_value: Input) -> JsonValue:
    if input_value["operation"] == "chat_text":
        chat_reply = input_value["response"]
        return {
            "text": chat_text(chat_reply),
            "finish_reason": finish_reason(chat_reply),
            "empty_error": empty_error(chat_reply, input_value["routed_model"]),
        }
    if input_value["operation"] == "responses_text":
        responses_reply = input_value["response"]
        return {"text": responses_text(responses_reply)}
    if input_value["operation"] == "call_cost":
        cost_payload = cost_response(input_value["response"])
        amount = call_cost(cost_payload)
        return {"cost": format(Decimal(repr(amount)).normalize(), "f")}
    if input_value["operation"] == "judge_shape":
        judge_reply = input_value["response"]
        return {"shape": judge_shape(judge_reply)}
    metadata = input_value["metadata"]
    decision = JSON.validate_python({**routing_decision(metadata)})
    classifier = classifier_cost(metadata)
    return {
        "decision": decision,
        "tier": routed_tier(metadata),
        "classifier_cost": format(Decimal(repr(classifier)).normalize(), "f"),
    }
