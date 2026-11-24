import asyncio
import inspect
from collections.abc import Awaitable, Sequence
from typing import Final, Protocol

import openai
import pytest
from completion_server import LITELLM, Dumpable, RefResult
from pydantic import JsonValue, TypeAdapter
from tuning_server import (
    AZURE_JOBS_PATH,
    BAD_KEY,
    CANCEL_PATH,
    GOOD_KEY,
    JOB,
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
from betterlite.fine_tuning._calls import HYPERPARAMETERS_DEFAULT
from betterlite.fine_tuning._page import FineTuningJobPage
from betterlite.fine_tuning.main import (
    acancel_fine_tuning_job,
    acreate_fine_tuning_job,
    alist_fine_tuning_jobs,
    aretrieve_fine_tuning_job,
    cancel_fine_tuning_job,
    create_fine_tuning_job,
    list_fine_tuning_jobs,
    retrieve_fine_tuning_job,
)
from betterlite.types.utils import LiteLLMFineTuningJob
from betterlite.types.utils.base import validated_value

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
AZURE_QUERY: Final = f"api-version={AZURE_VERSION}"
LIMIT: Final = 2
AFTER: Final = "ftjob-0"
MISSING_KEY_TEXT: Final = "Invalid API key provided: bad-key"
JOB_STATUS: Final = "running"
CANCELLED_STATUS: Final = "cancelled"
REFERENCE_CALLS: Final = 2
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
EXPECTED_CREATE_BODY: Final[dict[str, JsonValue]] = {
    "model": MODEL,
    "training_file": TRAINING_FILE,
    "hyperparameters": dict(HYPERPARAMETERS),
    "suffix": SUFFIX,
    "validation_file": VALIDATION_FILE,
    "integrations": list(INTEGRATIONS),
    "seed": SEED,
}
EXPECTED_AZURE_BODY: Final[dict[str, JsonValue]] = {
    "model": MODEL,
    "training_file": TRAINING_FILE,
    "hyperparameters": {"n_epochs": 2},
    "prompt_loss_weight": 0.1,
    "trainingType": 1,
}

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


# LiteLLM's call answers RefResult, and only a constructed model carries the job the caller gets.
def reference_dump(answer: RefResult) -> JsonValue:
    match answer:
        case Dumpable() as job:
            return JSON.validate_python(job.model_dump())
        case object():
            raise AssertionError("the reference call answered with no job")


def ours_dump(answer: LiteLLMFineTuningJob | Awaitable[LiteLLMFineTuningJob]) -> JsonValue:
    match answer:
        case LiteLLMFineTuningJob() as job:
            return JSON.validate_python(job.model_dump())
        case Awaitable():
            raise AssertionError("the sync call answered with a coroutine")


def page_of(answer: FineTuningJobPage | Awaitable[FineTuningJobPage]) -> FineTuningJobPage:
    match answer:
        case FineTuningJobPage() as page:
            return page
        case Awaitable():
            raise AssertionError("the sync list answered with a coroutine")


def job_of(
    answer: LiteLLMFineTuningJob | Awaitable[LiteLLMFineTuningJob],
) -> LiteLLMFineTuningJob:
    match answer:
        case LiteLLMFineTuningJob() as job:
            return job
        case Awaitable():
            raise AssertionError("the sync call answered with a coroutine")


# Both clients sent the same request; only create carries a JSON body.
def exchange_body(
    calls: Sequence[RecordedCall], method: str, path: str, query: str
) -> JsonValue | None:
    assert len(calls) == REFERENCE_CALLS
    first, second = calls
    assert first["method"] == second["method"] == method
    assert first["path"] == second["path"] == path
    assert first["query"] == second["query"] == query
    if not second["body"]:
        assert first["body"] == second["body"] == b""
        return None
    assert body_json(first) == body_json(second)
    return body_json(second)


# The four flag redirects, each answered by the coroutine its a-prefixed twin would return.
async def flagged_results(base: str) -> list[str]:
    results: list[str] = []
    created = create_fine_tuning_job(
        model=MODEL,
        training_file=TRAINING_FILE,
        acreate_fine_tuning_job=True,
        api_base=base,
        api_key=GOOD_KEY,
    )
    match created:
        case LiteLLMFineTuningJob():
            raise AssertionError("the flagged create answered with a job")
        case Awaitable() as created_pending:
            results.append((await created_pending).id)
    retrieved = retrieve_fine_tuning_job(
        JOB_ID, aretrieve_fine_tuning_job=True, api_base=base, api_key=GOOD_KEY
    )
    match retrieved:
        case LiteLLMFineTuningJob():
            raise AssertionError("the flagged retrieve answered with a job")
        case Awaitable() as retrieved_pending:
            results.append((await retrieved_pending).id)
    listed = list_fine_tuning_jobs(
        alist_fine_tuning_jobs=True, api_base=base, api_key=GOOD_KEY
    )
    match listed:
        case FineTuningJobPage():
            raise AssertionError("the flagged list answered with a page")
        case Awaitable() as listed_pending:
            page = await listed_pending
            results.append(",".join(job.id for job in page.data))
    cancelled = cancel_fine_tuning_job(
        JOB_ID, acancel_fine_tuning_job=True, api_base=base, api_key=GOOD_KEY
    )
    match cancelled:
        case LiteLLMFineTuningJob():
            raise AssertionError("the flagged cancel answered with a job")
        case Awaitable() as cancelled_pending:
            results.append((await cancelled_pending).status)
    return results


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
        reference = reference_dump(
            LITELLM.create_fine_tuning_job(
                model=MODEL,
                training_file=TRAINING_FILE,
                hyperparameters=dict(HYPERPARAMETERS),
                suffix=SUFFIX,
                validation_file=VALIDATION_FILE,
                integrations=JSON.validate_python(INTEGRATIONS),
                seed=SEED,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
        ours = ours_dump(
            create_fine_tuning_job(
                model=MODEL,
                training_file=TRAINING_FILE,
                hyperparameters=dict(HYPERPARAMETERS),
                suffix=SUFFIX,
                validation_file=VALIDATION_FILE,
                integrations=list(INTEGRATIONS),
                seed=SEED,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
        calls = server.calls
    assert ours == reference
    assert exchange_body(calls, "POST", JOBS_PATH, "") == EXPECTED_CREATE_BODY
    assert calls[1]["headers"]["authorization"] == f"Bearer {GOOD_KEY}"
    with TuningServer() as server:
        reference = reference_dump(
            LITELLM.create_fine_tuning_job(
                model=MODEL,
                training_file=TRAINING_FILE,
                hyperparameters=dict(AZURE_HYPERPARAMETERS),
                custom_llm_provider="azure",
                api_version=AZURE_VERSION,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
        ours = ours_dump(
            create_fine_tuning_job(
                model=MODEL,
                training_file=TRAINING_FILE,
                hyperparameters=dict(AZURE_HYPERPARAMETERS),
                custom_llm_provider="azure",
                api_version=AZURE_VERSION,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
        calls = server.calls
    assert ours == reference
    assert exchange_body(calls, "POST", AZURE_JOBS_PATH, AZURE_QUERY) == EXPECTED_AZURE_BODY
    assert calls[1]["headers"]["api-key"] == GOOD_KEY


def test_default_hyperparameters() -> None:
    source = dict(AZURE_HYPERPARAMETERS)
    with TuningServer() as server:
        azure_job = job_of(
            create_fine_tuning_job(
                model=MODEL,
                training_file=TRAINING_FILE,
                hyperparameters=source,
                custom_llm_provider="azure",
                api_version=AZURE_VERSION,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
        default_job = job_of(
            create_fine_tuning_job(
                model=MODEL,
                training_file=TRAINING_FILE,
                api_base=server.url,
                api_key=GOOD_KEY,
            )
        )
    assert azure_job.id == default_job.id == JOB_ID
    assert source == AZURE_HYPERPARAMETERS
    assert HYPERPARAMETERS_DEFAULT == {}


def test_retrieve_cancel_parity() -> None:
    with TuningServer() as server:
        reference = LITELLM.retrieve_fine_tuning_job(
            JOB_ID, api_base=server.url, api_key=GOOD_KEY
        )
        ours = retrieve_fine_tuning_job(JOB_ID, api_base=server.url, api_key=GOOD_KEY)
        assert ours_dump(ours) == reference_dump(reference)
        calls = server.calls
    assert exchange_body(calls, "GET", JOB_PATH, "") is None
    with TuningServer() as server:
        reference = LITELLM.cancel_fine_tuning_job(JOB_ID, api_base=server.url, api_key=GOOD_KEY)
        ours = cancel_fine_tuning_job(JOB_ID, api_base=server.url, api_key=GOOD_KEY)
        assert ours_dump(ours) == reference_dump(reference)
        calls = server.calls
    assert exchange_body(calls, "POST", CANCEL_PATH, "") is None


def test_list_matches_litellm() -> None:
    with TuningServer() as server:
        assert LITELLM.list_fine_tuning_jobs(
            after=AFTER, limit=LIMIT, api_base=server.url, api_key=GOOD_KEY
        ) is not None
        page = page_of(
            list_fine_tuning_jobs(
                after=AFTER, limit=LIMIT, api_base=server.url, api_key=GOOD_KEY
            )
        )
        calls = server.calls
    assert exchange_body(calls, "GET", JOBS_PATH, f"after={AFTER}&limit={LIMIT}") is None
    assert [job.id for job in page.data] == [JOB_ID, SECOND_JOB_ID]
    assert page.object == "list"
    assert page.has_more is False
    assert page.first_id == JOB_ID
    assert page.last_id == SECOND_JOB_ID
    assert page.data[0].status == JOB_STATUS


def test_list_null_error_to_none() -> None:
    job: JsonValue = {**JOB, "error": {"code": None, "message": None, "param": None}}
    page: JsonValue = {
        "object": "list",
        "data": [job],
        "has_more": False,
        "first_id": JOB_ID,
        "last_id": JOB_ID,
    }
    loaded = validated_value(page, FineTuningJobPage)
    assert loaded.data[0].error is None


def test_retrieve_list_flags() -> None:
    with TuningServer() as server:
        results = asyncio.run(flagged_results(server.url))
    assert results == [JOB_ID, JOB_ID, f"{JOB_ID},{SECOND_JOB_ID}", CANCELLED_STATUS]


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


def test_error_missing_azure_base(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("AZURE_API_BASE", raising=False)
    monkeypatch.delenv("AZURE_OPENAI_ENDPOINT", raising=False)
    with pytest.raises(ValueError, match="AZURE_OPENAI_ENDPOINT") as reference_caught:
        LITELLM.create_fine_tuning_job(
            model=MODEL,
            training_file=TRAINING_FILE,
            custom_llm_provider="azure",
            api_key=GOOD_KEY,
        )
    with pytest.raises(ValueError, match="AZURE_OPENAI_ENDPOINT") as ours_caught:
        create_fine_tuning_job(
            model=MODEL,
            training_file=TRAINING_FILE,
            custom_llm_provider="azure",
            api_key=GOOD_KEY,
        )
    assert str(ours_caught.value) == str(reference_caught.value)


def test_error_client_is_refused() -> None:
    with pytest.raises(UnsupportedCallArgument):
        create_fine_tuning_job(model=MODEL, training_file=TRAINING_FILE, client="client")
