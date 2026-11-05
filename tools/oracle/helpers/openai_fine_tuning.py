import asyncio
from collections.abc import Callable, Coroutine
from contextlib import ExitStack
from functools import partial
from typing import TypeAlias, TypedDict, cast
from unittest import mock

import httpx
import openai
import pydantic
from litellm.llms.openai.fine_tuning.handler import (
    OpenAIFineTuningAPI,
    _normalize_fine_tuning_job_dict,  # pyright: ignore[reportPrivateUsage]
)
from openai import AsyncAzureOpenAI, AsyncOpenAI, AzureOpenAI, OpenAI

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
SdkClient: TypeAlias = OpenAI | AsyncOpenAI | AzureOpenAI | AsyncAzureOpenAI

ENDPOINT = "https://fixture.test"
JOB_METHODS = ("create", "list", "retrieve", "cancel")


class Common(TypedDict):
    api_key: str | None
    api_base: str | None
    api_version: str | None
    timeout: float
    max_retries: int | None
    organization: str | None


def obj(value: Json) -> Object:
    if not isinstance(value, dict):
        raise TypeError("expected object")
    return value


def optional_obj(fields: Object, name: str) -> Object | None:
    value = fields.get(name)
    return None if value is None else obj(value)


def text(fields: Object, name: str) -> str | None:
    value = fields.get(name)
    if value is not None and not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def integer(fields: Object, name: str) -> int | None:
    value = fields.get(name)
    if value is not None and (isinstance(value, bool) or not isinstance(value, int)):
        raise TypeError(f"{name} must be an integer")
    return value


def number(fields: Object, name: str) -> float | None:
    value = fields.get(name)
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise TypeError(f"{name} must be a number")
    return float(value)


def flag(fields: Object, name: str) -> bool:
    value = fields.get(name, False)
    if not isinstance(value, bool):
        raise TypeError(f"{name} must be a boolean")
    return value


class Probe:
    def __init__(self, reply: Object | None) -> None:
        self.reply = reply
        self.stack = ExitStack()
        self.methods: list[tuple[str, mock.MagicMock]] = []
        self.constructed: list[Json] = []

    def respond(self, _request: httpx.Request) -> httpx.Response:
        if self.reply is None:
            return httpx.Response(200, json={})
        status = integer(self.reply, "status")
        return httpx.Response(status or 200, json=self.reply.get("body"))

    def client(self, kind: str) -> SdkClient:
        transport = httpx.MockTransport(self.respond)
        if kind == "sync":
            made: SdkClient = OpenAI(
                api_key="k",
                base_url=ENDPOINT,
                max_retries=0,
                http_client=httpx.Client(transport=transport),
            )
        elif kind == "async":
            made = AsyncOpenAI(
                api_key="k",
                base_url=ENDPOINT,
                max_retries=0,
                http_client=httpx.AsyncClient(transport=transport),
            )
        elif kind == "azure_sync":
            made = AzureOpenAI(
                api_key="k",
                api_version="2024-10-21",
                azure_endpoint=ENDPOINT,
                max_retries=0,
                http_client=httpx.Client(transport=transport),
            )
        elif kind == "azure_async":
            made = AsyncAzureOpenAI(
                api_key="k",
                api_version="2024-10-21",
                azure_endpoint=ENDPOINT,
                max_retries=0,
                http_client=httpx.AsyncClient(transport=transport),
            )
        else:
            raise ValueError(f"unknown client kind: {kind}")
        self.wrap(made)
        return made

    def wrap(self, made: SdkClient) -> None:
        jobs = made.fine_tuning.jobs
        for name in JOB_METHODS:
            wrapped = self.stack.enter_context(
                mock.patch.object(jobs, name, wraps=getattr(jobs, name))
            )  # pyright: ignore[reportAny]
            self.methods.append((name, wrapped))  # pyright: ignore[reportAny]

    def calls(self) -> Json:
        return [
            {name: dict(call.kwargs)}  # pyright: ignore[reportAny]
            for name, wrapped in self.methods
            for call in wrapped.call_args_list  # pyright: ignore[reportAny]
        ]


class Recorder:
    def __init__(
        self,
        probe: Probe,
        init: Callable[..., None],
        make_http: Callable[..., httpx.Client | httpx.AsyncClient],
    ) -> None:  # pyright: ignore[reportExplicitAny]
        self.probe = probe
        self.init = init
        self.make_http = make_http

    def __get__(self, instance: SdkClient, _owner: type) -> Callable[..., None]:  # pyright: ignore[reportExplicitAny]
        return partial(self.construct, instance)

    def construct(self, client: SdkClient, **kwargs: Json) -> None:
        self.probe.constructed.append({"class": type(client).__name__, "kwargs": kwargs})
        transport = httpx.MockTransport(self.probe.respond)
        self.init(
            client,
            api_key="k",
            base_url=ENDPOINT,
            max_retries=0,
            http_client=self.make_http(transport=transport),
        )
        self.probe.wrap(client)


def settle(result: object) -> Json:
    settled = asyncio.run(result) if isinstance(result, Coroutine) else result  # pyright: ignore[reportUnknownVariableType, reportUnknownArgumentType]
    if not isinstance(settled, pydantic.BaseModel):
        raise TypeError("expected a pydantic model")
    return cast(Json, settled.model_dump(mode="json"))


def common_arguments(args: Object) -> Common:
    return Common(
        api_key=text(args, "api_key"),
        api_base=text(args, "api_base"),
        api_version=text(args, "api_version"),
        timeout=cast(float, number(args, "timeout")),
        max_retries=integer(args, "max_retries"),
        organization=text(args, "organization"),
    )


def install_builder(probe: Probe, builder: str | None) -> None:
    if builder == "none":
        probe.stack.enter_context(
            mock.patch.object(OpenAIFineTuningAPI, "get_openai_client", return_value=None)
        )
    elif builder != "strict":
        probe.stack.enter_context(
            mock.patch.object(OpenAI, "__init__", Recorder(probe, OpenAI.__init__, httpx.Client))
        )
        probe.stack.enter_context(
            mock.patch.object(
                AsyncOpenAI, "__init__", Recorder(probe, AsyncOpenAI.__init__, httpx.AsyncClient)
            )
        )


def act(probe: Probe, step: Object) -> Json:
    action = text(step, "action")
    is_async = flag(step, "is_async")
    if action == "normalize":
        record = cast(dict[str, object], optional_obj(step, "record"))
        return cast(Json, _normalize_fine_tuning_job_dict(record, is_azure=flag(step, "is_azure")))
    kind = text(step, "client")
    supplied = None if kind is None else probe.client(kind)
    install_builder(probe, text(step, "builder"))
    common = common_arguments(optional_obj(step, "args") or {})
    api = OpenAIFineTuningAPI()
    job_id = text(step, "job_id") or ""
    if action == "project":
        built = api.get_openai_client(  # pyright: ignore[reportUnknownMemberType]
            client=supplied,
            _is_async=is_async,
            litellm_params=optional_obj(step, "litellm_params"),
            **common,
        )
        return (
            probe.constructed[0] if built is not None and probe.constructed else {"supplied": True}
        )
    if action == "create":
        data = {name: value for name, value in obj(step["data"]).items() if value is not None}
        created = api.create_fine_tuning_job(  # pyright: ignore[reportUnknownMemberType]
            _is_async=is_async, create_fine_tuning_job_data=data, client=supplied, **common
        )
        return settle(created)
    if action == "list":
        listed = api.list_fine_tuning_jobs(
            _is_async=is_async,
            client=supplied,
            after=text(step, "after"),
            limit=integer(step, "limit"),
            **common,
        )
        return settle(listed)
    if action == "retrieve":
        return settle(
            api.retrieve_fine_tuning_job(
                _is_async=is_async, fine_tuning_job_id=job_id, client=supplied, **common
            )
        )
    return settle(
        api.cancel_fine_tuning_job(
            _is_async=is_async, fine_tuning_job_id=job_id, client=supplied, **common
        )
    )


def outcome(probe: Probe, step: Object) -> Json:
    try:
        return {"returned": act(probe, step)}
    except pydantic.ValidationError:
        return {"raised": {"class": "ValidationError"}}
    except ValueError as error:
        return {"raised": {"class": "ValueError", "message": str(error)}}
    except TypeError:
        return {"raised": {"class": "TypeError"}}
    except openai.APIStatusError as error:
        return {
            "raised": {
                "class": "APIStatusError",
                "status": error.status_code,
                "type": error.type,
                "param": error.param,
                "code": error.code,
                "message": obj(cast(Json, error.body))["message"],
            }
        }


def run(argument: Json) -> Json:
    steps = obj(argument)["steps"]
    if not isinstance(steps, list):
        raise TypeError("steps must be a list")
    results: list[Json] = []
    for step in steps:
        probe = Probe(optional_obj(obj(step), "reply"))
        with probe.stack:
            result = outcome(probe, obj(step))
        results.append({"outcome": result, "calls": probe.calls()})
    return results
