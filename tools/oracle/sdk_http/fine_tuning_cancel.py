import asyncio
import json
import os
from collections.abc import Coroutine
from typing import Literal, TypeAlias

import litellm
import openai
import pydantic
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
ModelResult: TypeAlias = pydantic.BaseModel | Coroutine[object, object, pydantic.BaseModel]


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
        runs = [run_one(run, api_base) for run in self.fine_tuning_cancel.runs]
        return json.dumps({"fine_tuning_cancel": {"value": {"runs": runs}}})


def prepare_env(run: Run, api_base: str) -> None:
    variable = "AZURE_API_BASE" if run.custom_llm_provider == "azure" else "OPENAI_BASE_URL"
    os.environ[variable] = api_base if run.base == "env" else f"{api_base}/env-base"


def call_kwargs(run: Run, api_base: str) -> dict[str, object]:
    params = run.params
    kwargs: dict[str, object] = {
        "fine_tuning_job_id": run.fine_tuning_job_id,
        "custom_llm_provider": run.custom_llm_provider,
    }
    if run.base == "param":
        kwargs["api_base"] = api_base
    if run.extra_headers is not None:
        kwargs["extra_headers"] = run.extra_headers
    if run.extra_body is not None:
        kwargs["extra_body"] = run.extra_body
    if params.api_key is not None:
        kwargs["api_key"] = params.api_key
    if params.api_version is not None:
        kwargs["api_version"] = params.api_version
    if params.organization is not None:
        kwargs["organization"] = params.organization
    if params.timeout is not None:
        kwargs["timeout"] = params.timeout
    if params.request_timeout is not None:
        kwargs["request_timeout"] = params.request_timeout
    if params.max_retries is not None:
        kwargs["max_retries"] = params.max_retries
    if params.azure_ad_token is not None:
        kwargs["azure_ad_token"] = params.azure_ad_token
    return kwargs


def called(run: Run, api_base: str) -> ModelResult:
    kwargs = call_kwargs(run, api_base)
    if run.asynchronous:
        return litellm.acancel_fine_tuning_job(**kwargs)
    return litellm.cancel_fine_tuning_job(**kwargs)


def settle(result: ModelResult) -> JsonValue:
    match result:
        case Coroutine():
            with asyncio.Runner() as runner:
                settled = runner.run(result)
            return JSON.validate_json(settled.model_dump_json())
        case object():
            return JSON.validate_json(result.model_dump_json())


def status_failure(error: openai.APIStatusError) -> dict[str, JsonValue]:
    return {
        "class": type(error).__name__,
        "status": error.status_code,
        "type": error.type,
        "param": error.param,
        "code": error.code,
        "message": str(error),
    }


def plain_failure(error: ValueError | openai.OpenAIError) -> dict[str, JsonValue]:
    return {
        "class": type(error).__name__,
        "status": 0,
        "type": None,
        "param": None,
        "code": None,
        "message": str(error),
    }


def run_one(run: Run, api_base: str) -> dict[str, JsonValue]:
    prepare_env(run, api_base)
    try:
        job = settle(called(run, api_base))
    except openai.APIStatusError as error:
        return {"job": None, "error": status_failure(error)}
    except (ValueError, openai.OpenAIError) as error:
        return {"job": None, "error": plain_failure(error)}
    return {"job": job, "error": None}
