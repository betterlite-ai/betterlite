import base64
from collections.abc import Sequence
from typing import Literal, TypedDict

from litellm.proxy.guardrails.guardrail_hooks.model_armor.file_scanning import plan_file_scans
from litellm.types.llms.openai import AllMessageValues
from pydantic import JsonValue as Json


class PlanCall(TypedDict):
    function: Literal["plan_file_scans"]
    messages: Sequence[AllMessageValues]


def plan(call: PlanCall) -> Json:
    planned = plan_file_scans(call["messages"])
    attachments: list[Json] = [
        {
            "byte_data_type": attachment.byte_data_type,
            "file_bytes": base64.b64encode(attachment.file_bytes).decode("ascii"),
        }
        for attachment in planned.attachments
    ]
    return {"attachments": attachments, "unscannable_count": planned.unscannable_count}
