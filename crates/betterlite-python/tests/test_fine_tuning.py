import asyncio
import inspect
from collections.abc import Awaitable
from typing import Final, Protocol

import openai
import pytest
from completion_server import LITELLM, Dumpable
from pydantic import JsonValue, TypeAdapter
from tuning_server import (
    AZURE_JOBS_PATH,
    BAD_KEY,
    CANCEL_PATH,
    GOOD_KEY,
    JOB_ID,
    JOB_PATH,
    JOBS_PATH,
    SECOND_JOB_ID,
    RecordedCall,
    TuningServer,
)

from betterlite._completion.errors import UnsupportedCallArgument
from betterlite._native import NativeTuningError
from betterlite.exceptions import AuthenticationError
from betterlite.fine_tuning.main import (
    HYPERPARAMETERS_DEFAULT,
    acancel_fine_tuning_job,
    acreate_fine_tuning_job,
    alist_fine_tuning_jobs,
    aretrieve_fine_tuning_job,
    cancel_fine_tuning_job,
    create_fine_tuning_job,
    list_fine_tuning_jobs,
    retrieve_fine_tuning_job,
)
from betterlite.fine_tuning.types import FineTuningJobPage
from betterlite.types.utils import LiteLLMFineTuningJob

MODEL: Final = "gpt-4o-mini-2024-07-18"
TRAINING_FILE: Final = "file-abc"
VALIDATION_FILE: Final = "file-def"
SUFFIX: Final = "ft-1"
SEED: Final = 0
INTEGRATIONS: Final[list[str]] = ["azureml", "wandb"]
HYPERPARAMETERS: Final[dict[str, JsonValue]] = {"n_epochs": 2}
AZURE_HYPERPARAMETERS: Final[dict[str, JsonValue]] = {
    "n_epochs": 2,
    "prompt_loss_weight": 0.1,
}
AZURE_VERSION: Final = "2024-10-21"
LIMIT: Final = 2
AFTER: Final = "ftjob-0"
MISSING_KEY_TEXT: Final = "Invalid API key provided: bad-key"
JOB_STATUS: Final = "running"
CANCELLED_STATUS: Final = "cancelled"
REFERENCE_CALLS: Final = 2
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)

SIGNATURES: Final[list[tuple[str, inspect.Signature, inspect.Signature]]] = [
    (
        "create",
        inspect.signature(create_fine_tuning_job),
        inspect.signature(LITELLM.create_fine_tuning_job),
    ),
    (
        "acreate",
        inspect.signature(acreate_fine_tuning_job),
        inspect.signature(LITELLM.acreate_fine_tuning_job),
    ),
    (
        "retrieve",
        inspect.signature(retrieve_fine_tuning_job),
        inspect.signature(LITELLM.retrieve_fine_tuning_job),
    ),
    (
        "aretrieve",
        inspect.signature(aretrieve_fine_tuning_job),
        inspect.signature(LITELLM.aretrieve_fine_tuning_job),
    ),
    (
        "list",
        inspect.signature(list_fine_tuning_jobs),
        inspect.signature(LITELLM.list_fine_tuning_jobs),
    ),
    (
        "alist",
        inspect.signature(alist_fine_tuning_jobs),
        inspect.signature(LITELLM.alist_fine_tuning_jobs),
    ),
    (
        "cancel",
        inspect.signature(cancel_fine_tuning_job),
        inspect.signature(LITELLM.cancel_fine_tuning_job),
    ),
    (
        "acancel",
        inspect.signature(acancel_fine_tuning_job),
        inspect.signature(LITELLM.acancel_fine_tuning_job),
    ),
]


class DefaultView(Protocol):
    @property
    def default(self) -> JsonValue: ...


def parameter_rows(signature: inspect.Signature) -> list[tuple[str, str, str]]:
    rows: list[tuple[str, str, str]] = []
    for parameter in signature.parameters.values():
        default: DefaultView = parameter
        rows.append((parameter.name, parameter.kind.name, repr(default.default)))
    return rows


def body_json(record: RecordedCall) -> JsonValue:
    body = record["body"].decode()
    return JSON.validate_json(body)


async def awaited_job(
    answer: LiteLLMFineTuningJob | Awaitable[LiteLLMFineTuningJob],
) -> LiteLLMFineTuningJob:
    match answer:
        case LiteLLMFineTuningJob():
            raise AssertionError("the flagged call answered with a job, not a coroutine")
        case Awaitable():
            return await answer


async def awaited_page(
    answer: FineTuningJobPage | Awaitable[FineTuningJobPage],
) -> FineTuningJobPage:
    match answer:
        case FineTuningJobPage():
            raise AssertionError("the flagged call answered with a page, not a coroutine")
        case Awaitable():
            return await answer


def test_signatures_match_litellm() -> None:
    for name, ours, reference in SIGNATURES:
        assert parameter_rows(ours) == parameter_rows(reference), name
    assert inspect.iscoroutinefunction(acreate_fine_tuning_job)
    assert inspect.iscoroutinefunction(aretrieve_fine_tuning_job)
    assert inspect.iscoroutinefunction(alist_fine_tuning_jobs)
    assert inspect.iscoroutinefunction(acancel_fine_tuning_job)
    assert not inspect.iscoroutinefunction(create_fine_tuning_job)


def test_create_matches_litellm() -> None:
    with TuningServer() as server:
        match LITELLM.create_fine_tuning_job(
            model=MODEL,
            training_file=TRAINING_FILE,
            hyperparameters=dict(HYPERPARAMETERS),
            suffix=SUFFIX,
            validation_file=VALIDATION_FILE,
            integrations=JSON.validate_python(INTEGRATIONS),
            seed=SEED,
            api_base=server.url,
            api_key=GOOD_KEY,
        ):
            case Dumpable() as reference_job:
                reference = JSON.validate_python(reference_job.model_dump())
            case object():
                raise AssertionError("the reference create answered with no job")
        match create_fine_tuning_job(
            model=MODEL,
            training_file=TRAINING_FILE,
            hyperparameters=dict(HYPERPARAMETERS),
            suffix=SUFFIX,
            validation_file=VALIDATION_FILE,
            integrations=list(INTEGRATIONS),
            seed=SEED,
            api_base=server.url,
            api_key=GOOD_KEY,
        ):
            case LiteLLMFineTuningJob() as job:
                ours = JSON.validate_python(job.model_dump())
            case Awaitable():
                raise AssertionError("the sync create answered with a coroutine")
        calls = server.calls
    assert len(calls) == REFERENCE_CALLS
    first, second = calls
    assert first["method"] == second["method"] == "POST"
    assert first["path"] == second["path"] == JOBS_PATH
    assert first["query"] == second["query"] == ""
    assert second["headers"]["authorization"] == f"Bearer {GOOD_KEY}"
    assert first["headers"]["authorization"] == second["headers"]["authorization"]
    assert body_json(first) == body_json(second)
    expected: dict[str, JsonValue] = {
        "model": MODEL,
        "training_file": TRAINING_FILE,
        "hyperparameters": dict(HYPERPARAMETERS),
        "suffix": SUFFIX,
        "validation_file": VALIDATION_FILE,
        "integrations": list(INTEGRATIONS),
        "seed": SEED,
    }
    assert body_json(second) == expected
    assert ours == reference


def test_azure_create_parity() -> None:
    with TuningServer() as server:
        match LITELLM.create_fine_tuning_job(
            model=MODEL,
            training_file=TRAINING_FILE,
            hyperparameters=dict(AZURE_HYPERPARAMETERS),
            custom_llm_provider="azure",
            api_version=AZURE_VERSION,
            api_base=server.url,
            api_key=GOOD_KEY,
        ):
            case Dumpable() as reference_job:
                reference = JSON.validate_python(reference_job.model_dump())
            case object():
                raise AssertionError("the reference create answered with no job")
        match create_fine_tuning_job(
            model=MODEL,
            training_file=TRAINING_FILE,
            hyperparameters=dict(AZURE_HYPERPARAMETERS),
            custom_llm_provider="azure",
            api_version=AZURE_VERSION,
            api_base=server.url,
            api_key=GOOD_KEY,
        ):
            case LiteLLMFineTuningJob() as job:
                ours = JSON.validate_python(job.model_dump())
            case Awaitable():
                raise AssertionError("the sync create answered with a coroutine")
        calls = server.calls
    assert len(calls) == REFERENCE_CALLS
    first, second = calls
    assert first["method"] == second["method"] == "POST"
    assert first["path"] == second["path"] == AZURE_JOBS_PATH
    assert first["query"] == second["query"] == f"api-version={AZURE_VERSION}"
    assert first["headers"]["api-key"] == second["headers"]["api-key"] == GOOD_KEY
    assert body_json(first) == body_json(second)
    expected: dict[str, JsonValue] = {
        "model": MODEL,
        "training_file": TRAINING_FILE,
        "hyperparameters": {"n_epochs": 2},
        "prompt_loss_weight": 0.1,
        "trainingType": 1,
    }
    assert body_json(second) == expected
    assert ours == reference


def test_default_hyperparameters() -> None:
    assert HYPERPARAMETERS_DEFAULT == {}
    with TuningServer() as server:
        source = dict(AZURE_HYPERPARAMETERS)
        match create_fine_tuning_job(
            model=MODEL,
            training_file=TRAINING_FILE,
            hyperparameters=source,
            custom_llm_provider="azure",
            api_version=AZURE_VERSION,
            api_base=server.url,
            api_key=GOOD_KEY,
        ):
            case LiteLLMFineTuningJob():
                pass
            case Awaitable():
                raise AssertionError("the sync create answered with a coroutine")
        match create_fine_tuning_job(
            model=MODEL,
            training_file=TRAINING_FILE,
            api_base=server.url,
            api_key=GOOD_KEY,
        ):
            case LiteLLMFineTuningJob() as job:
                assert job.id == JOB_ID
            case Awaitable():
                raise AssertionError("the sync create answered with a coroutine")
    assert source == AZURE_HYPERPARAMETERS
    assert "prompt_loss_weight" in source
    assert HYPERPARAMETERS_DEFAULT == {}


def test_retrieve_matches_litellm() -> None:
    with TuningServer() as server:
        match LITELLM.retrieve_fine_tuning_job(
            fine_tuning_job_id=JOB_ID,
            api_base=server.url,
            api_key=GOOD_KEY,
        ):
            case Dumpable() as reference_job:
                reference = JSON.validate_python(reference_job.model_dump())
            case object():
                raise AssertionError("the reference retrieve answered with no job")
        match retrieve_fine_tuning_job(JOB_ID, api_base=server.url, api_key=GOOD_KEY):
            case LiteLLMFineTuningJob() as job:
                ours = JSON.validate_python(job.model_dump())
            case Awaitable():
                raise AssertionError("the sync retrieve answered with a coroutine")
        calls = server.calls
    assert len(calls) == REFERENCE_CALLS
    first, second = calls
    assert first["method"] == second["method"] == "GET"
    assert first["path"] == second["path"] == JOB_PATH
    assert first["query"] == second["query"] == ""
    assert first["body"] == second["body"] == b""
    assert ours == reference


def test_list_matches_litellm() -> None:
    with TuningServer() as server:
        assert LITELLM.list_fine_tuning_jobs(
            after=AFTER,
            limit=LIMIT,
            api_base=server.url,
            api_key=GOOD_KEY,
        ) is not None
        match list_fine_tuning_jobs(
            after=AFTER,
            limit=LIMIT,
            api_base=server.url,
            api_key=GOOD_KEY,
        ):
            case FineTuningJobPage() as page:
                listed = page
            case Awaitable():
                raise AssertionError("the sync list answered with a coroutine")
        calls = server.calls
    assert len(calls) == REFERENCE_CALLS
    first, second = calls
    assert first["method"] == second["method"] == "GET"
    assert first["path"] == second["path"] == JOBS_PATH
    assert first["query"] == second["query"] == f"after={AFTER}&limit={LIMIT}"
    assert first["body"] == second["body"] == b""
    match listed.data[0]:
        case LiteLLMFineTuningJob() as first_job:
            assert first_job.id == JOB_ID
    assert [job.id for job in listed.data] == [JOB_ID, SECOND_JOB_ID]
    assert listed.object == "list"
    assert listed.has_more is False
    assert listed.first_id == JOB_ID
    assert listed.last_id == SECOND_JOB_ID
    assert listed.data[0].status == JOB_STATUS


def test_cancel_matches_litellm() -> None:
    with TuningServer() as server:
        match LITELLM.cancel_fine_tuning_job(
            fine_tuning_job_id=JOB_ID,
            api_base=server.url,
            api_key=GOOD_KEY,
        ):
            case Dumpable() as reference_job:
                reference = JSON.validate_python(reference_job.model_dump())
            case object():
                raise AssertionError("the reference cancel answered with no job")
        match cancel_fine_tuning_job(JOB_ID, api_base=server.url, api_key=GOOD_KEY):
            case LiteLLMFineTuningJob() as job:
                ours = JSON.validate_python(job.model_dump())
            case Awaitable():
                raise AssertionError("the sync cancel answered with a coroutine")
        calls = server.calls
    assert len(calls) == REFERENCE_CALLS
    first, second = calls
    assert first["method"] == second["method"] == "POST"
    assert first["path"] == second["path"] == CANCEL_PATH
    assert first["query"] == second["query"] == ""
    assert first["body"] == second["body"] == b""
    assert ours == reference


def test_retrieve_list_flags() -> None:
    with TuningServer() as server:
        created = create_fine_tuning_job(
            model=MODEL,
            training_file=TRAINING_FILE,
            acreate_fine_tuning_job=True,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        assert asyncio.run(awaited_job(created)).id == JOB_ID
        retrieved = retrieve_fine_tuning_job(
            JOB_ID,
            aretrieve_fine_tuning_job=True,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        assert asyncio.run(awaited_job(retrieved)).id == JOB_ID
        listed = list_fine_tuning_jobs(
            alist_fine_tuning_jobs=True,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        page = asyncio.run(awaited_page(listed))
        assert [job.id for job in page.data] == [JOB_ID, SECOND_JOB_ID]
        cancelled = cancel_fine_tuning_job(
            JOB_ID,
            acancel_fine_tuning_job=True,
            api_base=server.url,
            api_key=GOOD_KEY,
        )
        assert asyncio.run(awaited_job(cancelled)).status == CANCELLED_STATUS


def test_error_class_and_cause() -> None:
    with TuningServer() as server:
        with pytest.raises(openai.AuthenticationError) as reference_caught:
            LITELLM.create_fine_tuning_job(
                model=MODEL,
                training_file=TRAINING_FILE,
                api_base=server.url,
                api_key=BAD_KEY,
            )
        with pytest.raises(AuthenticationError) as ours_caught:
            create_fine_tuning_job(
                model=MODEL,
                training_file=TRAINING_FILE,
                api_base=server.url,
                api_key=BAD_KEY,
            )
    assert ours_caught.value.status_code == reference_caught.value.status_code
    assert MISSING_KEY_TEXT in str(ours_caught.value)
    assert MISSING_KEY_TEXT in str(reference_caught.value)
    match ours_caught.value.__cause__:
        case NativeTuningError():
            pass
        case BaseException() | None:
            raise AssertionError("the provider error kept no native cause")


def test_error_client_is_refused() -> None:
    with pytest.raises(UnsupportedCallArgument):
        create_fine_tuning_job(model=MODEL, training_file=TRAINING_FILE, client="client")
