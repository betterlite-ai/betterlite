import asyncio
from typing import TYPE_CHECKING, Literal, NotRequired, TypeAlias, TypedDict, cast
from unittest.mock import patch

import litellm
from fastapi import HTTPException
from litellm.proxy.guardrails.guardrail_hooks.llm_as_a_judge import (
    initialize_guardrail,
)
from litellm.types.guardrails import Guardrail, LitellmParams

if TYPE_CHECKING:
    from litellm.types.utils import GenericGuardrailAPIInputs

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

JUDGE_CALL_TARGET = (
    "litellm.proxy.guardrails.guardrail_hooks.llm_as_a_judge.judge_acompletion"
)


class JudgeOutcome(TypedDict, total=False):
    content: str | None
    error: str


class GuardrailConfig(TypedDict):
    guardrail_name: str
    litellm_params: dict[str, Json]


class Input(TypedDict):
    guardrail: GuardrailConfig
    inputs: dict[str, Json]
    request_data: NotRequired[dict[str, Json]]
    input_type: Literal["request", "response"]
    judge: NotRequired[JudgeOutcome]


class _FakeJudge:
    def __init__(self, captured: dict[str, Json], outcome: JudgeOutcome) -> None:
        self.captured = captured
        self.outcome = outcome

    async def __call__(
        self,
        _router: object,
        model: str,
        messages: list[dict[str, Json]],
        **kwargs: Json,
    ) -> litellm.ModelResponse:
        self.captured["model"] = model
        self.captured["messages"] = cast(Json, messages)
        for name, value in kwargs.items():
            self.captured[name] = value
        error = self.outcome.get("error")
        if error is not None:
            raise RuntimeError(error)
        return litellm.ModelResponse(
            choices=[
                {
                    "message": {
                        "role": "assistant",
                        "content": self.outcome.get("content"),
                    }
                }
            ]
        )


def run(input_value: dict[str, Json]) -> Json:
    fields = cast(Input, input_value)
    config = fields["guardrail"]
    parameters = LitellmParams(**config["litellm_params"])
    guardrail_entry = cast(
        Guardrail,
        {"guardrail_name": config["guardrail_name"], "litellm_params": parameters},
    )
    guardrail = initialize_guardrail(parameters, guardrail_entry)
    captured: dict[str, Json] = {}
    outcome = fields.get("judge") or {}
    request_data = fields.get("request_data") or {}
    inputs = cast("GenericGuardrailAPIInputs", fields["inputs"])
    blocked: Json = None
    result: Json = None
    with patch(JUDGE_CALL_TARGET, new=_FakeJudge(captured, outcome)):
        try:
            returned = asyncio.run(
                guardrail.apply_guardrail(  # pyright: ignore[reportUnknownMemberType]
                    inputs=inputs,
                    request_data=request_data,
                    input_type=fields["input_type"],
                )
            )
            result = cast(Json, returned)
        except HTTPException as error:
            blocked = {
                "status": error.status_code,
                "detail": cast(Json, error.detail),
            }
    metadata = cast(dict[str, Json], request_data.get("metadata") or {})
    entries = cast(
        list[dict[str, Json]],
        metadata.get("standard_logging_guardrail_information") or [],
    )
    records: list[Json] = [
        {
            "guardrail_name": entry["guardrail_name"],
            "guardrail_provider": entry["guardrail_provider"],
            "guardrail_mode": entry["guardrail_mode"],
            "guardrail_response": entry["guardrail_response"],
            "guardrail_status": entry["guardrail_status"],
        }
        for entry in entries
    ]
    return {
        "judge_call": captured or None,
        "inputs": result,
        "eval_information": metadata.get("eval_information"),
        "blocked": blocked,
        "guardrail_records": records or None,
    }
