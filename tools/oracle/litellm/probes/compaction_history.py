from collections.abc import Callable, Mapping
from typing import Literal

from litellm.exceptions import BadRequestError, ContextWindowExceededError
from litellm.router_strategy.complexity_router import context_compaction
from litellm.router_strategy.complexity_router.config import ContextCompactionConfig
from litellm.router_strategy.complexity_router.context_compaction import CompactionState
from pydantic import Field, JsonValue, TypeAdapter
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

_HISTORY_MESSAGE: str = "litellm.ContextWindowExceededError: litellm.BadRequestError: Context compaction: "
_REQUEST_MESSAGE: str = "litellm.BadRequestError: Context compaction: "

_PORTABLE_HISTORY: Callable[
    ...,
    tuple[
        tuple[dict[str, JsonValue], ...],
        tuple[dict[str, JsonValue], ...],
        tuple[dict[str, JsonValue], ...],
    ],
] = TypeAdapter(
    Callable[
        ...,
        tuple[
            tuple[dict[str, JsonValue], ...],
            tuple[dict[str, JsonValue], ...],
            tuple[dict[str, JsonValue], ...],
        ],
    ]
).validate_python(context_compaction.__dict__["_portable_history"])

_TOOL_IDS: Callable[..., tuple[str, ...]] = TypeAdapter(
    Callable[..., tuple[str, ...]]
).validate_python(context_compaction.__dict__["_tool_ids"])

_COMPACTOR_INPUT: Callable[..., Mapping[str, JsonValue]] = TypeAdapter(
    Callable[..., Mapping[str, JsonValue]]
).validate_python(context_compaction.__dict__["_compactor_input"])

_COMPACTOR_MODEL: Callable[..., tuple[str, object]] = TypeAdapter(
    Callable[..., tuple[str, object]]
).validate_python(context_compaction.__dict__["_compactor_model"])


class DeploymentInput(Strict):
    litellm_params: dict[str, JsonValue]
    max_input_tokens: int | None = None
    max_output_tokens: int | None = None


class GroupInput(Strict):
    model: str
    deployments: list[DeploymentInput] = Field(default_factory=list)


class HistoryOutput(Strict):
    function: Literal["history"] = "history"
    outcome: Literal["ok", "unavailable"]
    instructions: int
    prefix: int
    tail: int
    calls: list[str]
    results: list[str]
    reason: str | None
    status: int | None
    code: str | None
    kind: Literal["context_window_exceeded", "bad_request"] | None


class CompactorOutput(Strict):
    function: Literal["compactor"] = "compactor"
    outcome: Literal["ok", "unavailable"]
    model: str | None
    request: dict[str, JsonValue] | None
    reason: str | None
    status: int | None
    code: str | None
    kind: Literal["context_window_exceeded", "bad_request"] | None


class ProbeRouter:
    def __init__(self, groups: list[GroupInput], count: int) -> None:
        self.groups: dict[str, GroupInput] = {group.model: group for group in groups}
        self.count: int = count

    def get_model_list(self, model_name: str) -> list[dict[str, JsonValue]] | None:
        group = self.groups.get(model_name)
        if group is None:
            return None
        return [
            {
                "litellm_params": deployment.litellm_params,
                "max_input_tokens": deployment.max_input_tokens,
                "max_output_tokens": deployment.max_output_tokens,
            }
            for deployment in group.deployments
        ]

    def get_router_model_info(
        self, deployment: dict[str, JsonValue], received_model_name: str
    ) -> dict[str, JsonValue]:
        _ = received_model_name
        return deployment

    def _count_pre_call_check_tokens(self, **_kwargs: JsonValue) -> int:
        return self.count


class CompactionHistory(Operation):
    function: Literal["history", "compactor"]
    model: str | None = None
    surface: Literal["chat", "messages", "responses"] | None = None
    messages: list[dict[str, JsonValue]] = Field(default_factory=list)
    items: list[JsonValue] = Field(default_factory=list)
    tools: list[JsonValue] = Field(default_factory=list)
    text: str | None = None
    config_model: str | None = None
    max_tokens: int | None = None
    count: int | None = None
    groups: list[GroupInput] = Field(default_factory=list)

    @override
    def run(self) -> Strict:
        if self.function == "history":
            return self._history()
        return self._compactor()

    def _surface(self) -> Literal["chat", "messages", "responses"]:
        if self.surface is None:
            raise ProbeError("invalid_request", "the compaction probe needs a surface")
        return self.surface

    def _payload(self) -> dict[str, object]:
        model = self.model or ""
        if self.surface == "responses":
            raw: object = self.items if self.text is None else self.text
            return {"model": model, "input": raw, "tools": self.tools}
        raw = self.messages if self.text is None else self.text
        return {"model": model, "messages": raw}

    def _history(self) -> HistoryOutput:
        surface = self._surface()
        payload = self._payload()
        try:
            instructions, prefix, tail = _PORTABLE_HISTORY(payload, surface)
        except ContextWindowExceededError as error:
            return HistoryOutput(
                outcome="unavailable",
                instructions=0,
                prefix=0,
                tail=0,
                calls=[],
                results=[],
                reason=str(error.message).removeprefix(_HISTORY_MESSAGE),
                status=400,
                code="400",
                kind="context_window_exceeded",
            )
        return HistoryOutput(
            outcome="ok",
            instructions=len(instructions),
            prefix=len(prefix),
            tail=len(tail),
            calls=list(_TOOL_IDS(prefix, results=False)),
            results=list(_TOOL_IDS(prefix, results=True)),
            reason=None,
            status=None,
            code=None,
            kind=None,
        )

    def _compactor(self) -> CompactorOutput:
        surface = self._surface()
        if surface == "responses":
            raise ProbeError(
                "invalid_request", "the compactor operation reads only chat or messages history"
            )
        if self.max_tokens is None:
            raise ProbeError("invalid_request", "the compactor operation needs max_tokens")
        payload: dict[str, object] = {"model": self.model or "", "messages": self.messages}
        config = ContextCompactionConfig(model=self.config_model, max_tokens=self.max_tokens)
        state = CompactionState(
            config=config, candidates=tuple(group.model for group in self.groups)
        )
        request: dict[str, JsonValue] | None = None
        try:
            instructions, prefix, _tail = _PORTABLE_HISTORY(payload, surface)
            request = dict(
                _COMPACTOR_INPUT(payload, surface, instructions, prefix, config.max_tokens)
            )
            selected, _provider = _COMPACTOR_MODEL(
                ProbeRouter(self.groups, 0 if self.count is None else self.count), state, request
            )
        except ContextWindowExceededError as error:
            return CompactorOutput(
                outcome="unavailable",
                model=None,
                request=request,
                reason=str(error.message).removeprefix(_HISTORY_MESSAGE),
                status=400,
                code="400",
                kind="context_window_exceeded",
            )
        except BadRequestError as error:
            return CompactorOutput(
                outcome="unavailable",
                model=None,
                request=request,
                reason=str(error.message).removeprefix(_REQUEST_MESSAGE),
                status=400,
                code="400",
                kind="bad_request",
            )
        return CompactorOutput(
            outcome="ok",
            model=selected,
            request=request,
            reason=None,
            status=None,
            code=None,
            kind=None,
        )
