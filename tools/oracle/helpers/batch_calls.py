from collections.abc import Callable, Iterable
from concurrent.futures import Future, wait
from types import TracebackType
from typing import Self, TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.batch_completion import main as batch_main

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class ProbeError(Exception):
    pass


class ControlledPool:
    def __init__(self, outcomes: list[dict[str, Json]], order: list[int], *, race: bool) -> None:
        self.outcomes = outcomes
        self.order = iter(order)
        self.race = race
        self.futures: list[Future[Json]] = []

    def __enter__(self) -> Self:
        return self

    def __exit__(
        self,
        kind: type[BaseException] | None,
        error: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        for index in self.order:
            self.complete(index)

    def complete(self, index: int) -> None:
        outcome = self.outcomes[index]
        error = outcome.get("error")
        if isinstance(error, str):
            self.futures[index].set_exception(ProbeError(error))
        else:
            self.futures[index].set_result(outcome.get("value"))

    def submit(
        self, function: Callable[..., object], *args: object, **kwargs: object
    ) -> Future[Json]:
        if function is not litellm.completion:
            raise ProbeError("expected the single-call completion entry point")
        del args, kwargs
        future: Future[Json] = Future()
        self.futures.append(future)
        if not self.race and len(self.futures) == len(self.outcomes):
            for index in self.order:
                self.complete(index)
        return future

    def wait(
        self, futures: Iterable[Future[Json]], *, return_when: str
    ) -> tuple[set[Future[Json]], set[Future[Json]]]:
        self.complete(next(self.order))
        return wait(futures, return_when=return_when)


def run(input_value: dict[str, Json]) -> Json:
    outcomes = cast(list[dict[str, Json]], input_value.get("results", []))
    order = cast(list[int], input_value.get("completion_order", list(range(len(outcomes)))))
    mode = input_value["mode"]
    pool = ControlledPool(outcomes, order, race=mode == "deployments")
    models = cast(list[str], input_value.get("models", []))
    messages = [[{"role": "user", "content": str(index)}] for index in range(len(outcomes))]
    with (
        patch.object(batch_main, "ThreadPoolExecutor", return_value=pool),
        patch("concurrent.futures.ThreadPoolExecutor", return_value=pool),
        patch.object(batch_main, "wait", side_effect=pool.wait),
    ):
        if mode == "messages":
            call = cast(Callable[..., list[object]], batch_main.batch_completion)
            values = call(model="mock", messages=messages)
            return [None if isinstance(value, Exception) else cast(Json, value) for value in values]
        if mode == "all_responses":
            call_all = cast(Callable[..., Json], batch_main.batch_completion_models_all_responses)
            return call_all(models=models)
        call_models = cast(Callable[..., Json], batch_main.batch_completion_models)
        if mode == "models":
            return {"selected": call_models(models=models)}
        deployments = [{"model": model} for model in models]
        return {"selected": call_models(deployments=deployments, model_list=[])}
