import asyncio
from typing import Literal, NotRequired, TypeAlias, TypedDict, cast

from fastapi import HTTPException
from litellm.proxy.guardrails.guardrail_hooks import llm_as_a_judge
from litellm.proxy.guardrails.guardrail_hooks.llm_as_a_judge import initialize_guardrail
from litellm.router import Router
from litellm.types.guardrails import LitellmParams
from litellm.types.utils import ModelResponse

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

RECORD_KEYS = ("guardrail_name", "guardrail_provider", "guardrail_mode", "guardrail_response",
               "guardrail_status", "start_time", "end_time")


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
        _router: Router | None,
        judge_model: str,
        messages: list[dict[str, Json]],
        **params: Json,
    ) -> ModelResponse:
        self.captured["model"] = judge_model
        self.captured["messages"] = cast(Json, messages)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/
        for name, value in params.items():
            self.captured[name] = value
        error = self.outcome.get("error")
        if error is not None:
            raise RuntimeError(error)
        content = self.outcome.get("content")
        choice = {"message": {"role": "assistant", "content": content}}
        return ModelResponse(choices=[choice])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/


def run(input_value: Input) -> Json:
    config = input_value["guardrail"]
    parameters = LitellmParams(**config["litellm_params"])
    entry = {"guardrail_name": config["guardrail_name"], "litellm_params": parameters}
    guardrail = initialize_guardrail(parameters, entry)  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/
    captured: dict[str, Json] = {}
    outcome = input_value.get("judge")
    if outcome is None:
        outcome = JudgeOutcome()
    request_data = input_value.get("request_data")
    if request_data is None:
        request_data = {}
    blocked: Json = None
    result: Json = None
    fake = _FakeJudge(captured, outcome)
    llm_as_a_judge.judge_acompletion = fake  # type: ignore[attr-defined, assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/
    try:
        inputs = input_value["inputs"]
        input_type = input_value["input_type"]
        coro = guardrail.apply_guardrail(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/
            inputs,  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/
            request_data,
            input_type,
        )
        returned = asyncio.run(coro)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/
        result = cast(Json, returned)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/
    except HTTPException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/
        blocked = {
            "status": error.status_code,
            "detail": cast(Json, error.detail),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/
        }
    metadata = request_data.get("metadata")
    entries_value = None
    eval_information = None
    if metadata is not None:
        info = cast(dict[str, Json], metadata)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/
        entries_value = info.get("standard_logging_guardrail_information")
        eval_information = info.get("eval_information")
    entries = [] if entries_value is None else cast(list[dict[str, Json]], entries_value)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/
    records = [{key: entry[key] for key in RECORD_KEYS} for entry in entries]
    return {
        "judge_call": captured if len(captured) > 0 else None,
        "inputs": result,
        "eval_information": eval_information,
        "blocked": blocked,
        "guardrail_records": None if len(records) == 0 else cast(Json, records),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/
    }
