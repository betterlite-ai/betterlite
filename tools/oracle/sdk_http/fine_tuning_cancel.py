import asyncio
import json
import os
from collections.abc import Coroutine
from typing import Literal, TypeAlias, TypedDict

import litellm
import openai
import pydantic
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
ModelResult: TypeAlias = pydantic.BaseModel | Coroutine[object, object, pydantic.BaseModel]


class CallKwargs(TypedDict):
    fine_tuning_job_id: str
    custom_llm_provider: str | None
    api_base: str | None
    extra_headers: dict[str, str] | None
    extra_body: dict[str, str] | None
    api_key: str | None
    api_version: str | None
    organization: str | None
    timeout: float | str | None
    request_timeout: float | None
    max_retries: int | None
    azure_ad_token: str | None


class Params(BaseModel):
    model_config = ConfigDict(extra="forbid")

    api_key: str | None = None
    api_version: str | None = None
    organization: str | None = None
    timeout: float | str | None = None
    request_timeout: float | None = None
    max_retries: int | None = None
    azure_ad_token: str | None = None


class Run(BaseModel):
    model_config = ConfigDict(extra="forbid")

    fine_tuning_job_id: str
    custom_llm_provider: str | None = "openai"
    base: Literal["param", "env"] = "param"
    asynchronous: bool = False
    extra_headers: dict[str, str] | None = None
    extra_body: dict[str, str] | None = None
    params: Params = Field(default_factory=Params)


class FineTuningCancelCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    runs: list[Run]


class FineTuningCancelRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    fine_tuning_cancel: FineTuningCancelCall

    def serialize(self, api_base: str) -> str:
        runs: list[JsonValue] = [run_one(run, api_base) for run in self.fine_tuning_cancel.runs]
        outcome: JsonValue = {"fine_tuning_cancel": {"value": {"runs": runs}}}
        return json.dumps(outcome)


def prepare_env(run: Run, api_base: str) -> None:
    variable = "AZURE_API_BASE" if run.custom_llm_provider == "azure" else "OPENAI_BASE_URL"
    os.environ[variable] = api_base if run.base == "env" else f"{api_base}/env-base"


def call_kwargs(run: Run, api_base: str) -> CallKwargs:
    params = run.params
    return {
        "fine_tuning_job_id": run.fine_tuning_job_id,
        "custom_llm_provider": run.custom_llm_provider,
        "api_base": api_base if run.base == "param" else None,
        "extra_headers": run.extra_headers,
        "extra_body": run.extra_body,
        "api_key": params.api_key,
        "api_version": params.api_version,
        "organization": params.organization,
        "timeout": params.timeout,
        "request_timeout": params.request_timeout,
        "max_retries": params.max_retries,
        "azure_ad_token": params.azure_ad_token,
    }


def called(run: Run, api_base: str) -> ModelResult:
    kwargs = call_kwargs(run, api_base)
    if run.asynchronous:
        pending: ModelResult = litellm.acancel_fine_tuning_job(**kwargs)
        return pending
    answer: ModelResult = litellm.cancel_fine_tuning_job(**kwargs)
    return answer


def settle(result: ModelResult) -> JsonValue:
    match result:
        case Coroutine():
            with asyncio.Runner() as runner:
                settled = runner.run(result)
            return JSON.validate_json(settled.model_dump_json())
        case object():
            return JSON.validate_json(result.model_dump_json())


def failure(
    error: ValueError | openai.OpenAIError, details: openai.APIStatusError | None
) -> dict[str, JsonValue]:
    status = 0 if details is None else details.status_code
    return {
        "class": type(error).__name__,
        "status": status,
        "type": None if details is None else details.type,
        "param": None if details is None else details.param,
        "code": None if details is None else details.code,
        "message": str(error),
    }


def run_one(run: Run, api_base: str) -> dict[str, JsonValue]:
    prepare_env(run, api_base)
    try:
        job = settle(called(run, api_base))
    except openai.APIStatusError as error:
        return {"job": None, "error": failure(error, error)}
    except (ValueError, openai.OpenAIError) as error:
        return {"job": None, "error": failure(error, None)}
    return {"job": job, "error": None}
