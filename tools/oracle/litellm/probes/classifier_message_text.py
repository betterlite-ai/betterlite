import asyncio
from collections.abc import Mapping, Sequence
from typing import Final

from litellm.router import Router
from litellm.router_strategy.complexity_router.complexity_router import (
    ClassificationOutcome,
    ComplexityRouter,
)
from litellm.types.router import PreRoutingStrategy
from pydantic import Field, JsonValue
from typing_extensions import override

from probes.protocol import Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

CLASSIFIER_MODEL: Final = "classifier-message-text"
# a floor below the top configured tier: a top floor makes plan-mode routing answer without
# classifying, and the classifier call is where routing hands over the text this probe reports
PLAN_MODE_FLOOR: Final = "SIMPLE"


class MarkerPairInput(Strict):
    open: str
    close: str


class MessageInput(Strict):
    role: str
    content: JsonValue | None = None


class Output(Strict):
    ask: str | None
    system_prompt: str | None
    markers: list[MarkerPairInput]
    plan_sentinel: str | None


class ClassifyCapturedError(Exception):
    def __init__(self, prompt: str, system_prompt: str | None) -> None:
        super().__init__(prompt)
        self.prompt: str = prompt
        self.system_prompt: str | None = system_prompt


# typed as the protocol the router calls with: the concrete method annotates request_kwargs as a
# bare dict, which a strict type check sees as a partially unknown signature
def drive_pre_routing_hook(
    strategy: PreRoutingStrategy,
    messages: list[dict[str, object]],
    request_kwargs: dict[str, object],
) -> None:
    _ = asyncio.run(
        strategy.async_pre_routing_hook(
            model=CLASSIFIER_MODEL,
            request_kwargs=request_kwargs,
            messages=messages,
        )
    )


class ProbeRouter(ComplexityRouter):
    @override
    async def aclassify(
        self,
        prompt: str,
        system_prompt: str | None = None,
        request_kwargs: dict[str, object] | None = None,
        messages: Sequence[Mapping[str, object]] | None = None,
        raw_messages: list[dict[str, object]] | None = None,
    ) -> ClassificationOutcome:
        raise ClassifyCapturedError(prompt, system_prompt)

    def ask_and_system_prompt(
        self,
        messages: list[dict[str, object]],
        request_kwargs: dict[str, object],
    ) -> tuple[str | None, str | None]:
        try:
            drive_pre_routing_hook(self, messages, request_kwargs)
        except ClassifyCapturedError as captured:
            return captured.prompt, captured.system_prompt
        # routing returns without classifying only when no human ask survives extraction, and the
        # classifier call is the only place routing hands the extracted system prompt to a caller
        return None, None

    def selected_markers(self, request_kwargs: dict[str, object]) -> tuple[tuple[str, str], ...]:
        return self._reminder_markers_for_request(request_kwargs)

    def plan_sentinel(
        self,
        request_kwargs: dict[str, object],
        messages: list[dict[str, object]],
    ) -> str | None:
        return self._matched_plan_mode_signal(request_kwargs, messages)


class ClassifierMessageText(Operation):
    messages: list[MessageInput] = Field(default_factory=list)
    reminder_markers: list[MarkerPairInput] | None = None
    user_agent: str | None = None
    body: JsonValue | None = None
    plan_mode_patterns: list[str] = Field(default_factory=list)

    def router(self) -> ProbeRouter:
        config: dict[str, JsonValue] = {
            "plan_mode_min_tier": PLAN_MODE_FLOOR,
            "plan_mode_patterns": list(self.plan_mode_patterns),
        }
        if self.reminder_markers is not None:
            config["reminder_markers"] = [
                {"open": pair.open, "close": pair.close} for pair in self.reminder_markers
            ]
        return ProbeRouter(
            model_name=CLASSIFIER_MODEL,
            litellm_router_instance=Router(),
            complexity_router_config=config,
        )

    @override
    def run(self) -> Output:
        router = self.router()
        messages: list[dict[str, object]] = [
            {"role": message.role, "content": message.content} for message in self.messages
        ]
        request_kwargs: dict[str, object] = {"proxy_server_request": {"body": self.body}}
        if self.user_agent is not None:
            request_kwargs["metadata"] = {"user_agent": self.user_agent}
        markers = router.selected_markers(request_kwargs)
        ask, system_prompt = router.ask_and_system_prompt(messages, request_kwargs)
        return Output(
            ask=ask,
            system_prompt=system_prompt,
            markers=[
                MarkerPairInput(open=open_marker, close=close_marker)
                for open_marker, close_marker in markers
            ],
            plan_sentinel=router.plan_sentinel(request_kwargs, messages),
        )
