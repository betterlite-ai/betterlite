import asyncio
from typing import TypeAlias

from litellm.proxy.guardrails.guardrail_hooks.headroom.headroom import HeadroomGuardrail
from pydantic import JsonValue

from b3775 import headroom, hiddenlayer
from b3775.service import HiddenlayerInput, PlanInput, ShouldRunInput

Input: TypeAlias = ShouldRunInput | PlanInput | HiddenlayerInput


def run(input_value: Input) -> JsonValue:
    match input_value["kind"]:
        case "should_run":
            agent: HeadroomGuardrail = headroom.guardrail(None, None)
            return asyncio.run(headroom.gate(input_value, agent))
        case "plan":
            return headroom.plan(input_value)
        case "hiddenlayer":
            return hiddenlayer.run(input_value)
