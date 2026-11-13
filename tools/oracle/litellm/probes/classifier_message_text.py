from typing import Final

from litellm.router import Router
from litellm.router_strategy.complexity_router.complexity_router import ComplexityRouter
from pydantic import Field, JsonValue
from typing_extensions import override

from probes.protocol import Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

PLAN_MODE_FLOOR: Final = "COMPLEX"


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


class ProbeRouter(ComplexityRouter):
    def ask_and_system_prompt(
        self, messages: list[dict[str, object]]
    ) -> tuple[str | None, str | None]:
        return self._extract_user_message_and_system_prompt(messages)

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
            model_name="classifier-message-text",
            litellm_router_instance=Router(),
            complexity_router_config=config,
        )

    @override
    def run(self) -> Output:
        router = self.router()
        messages: list[dict[str, object]] = [
            {"role": message.role, "content": message.content} for message in self.messages
        ]
        ask, system_prompt = router.ask_and_system_prompt(messages)
        marker_kwargs: dict[str, object] = {}
        if self.user_agent is not None:
            marker_kwargs["metadata"] = {"user_agent": self.user_agent}
        markers = router.selected_markers(marker_kwargs)
        request_kwargs: dict[str, object] = {"proxy_server_request": {"body": self.body}}
        return Output(
            ask=ask,
            system_prompt=system_prompt,
            markers=[
                MarkerPairInput(open=open_marker, close=close_marker)
                for open_marker, close_marker in markers
            ],
            plan_sentinel=router.plan_sentinel(request_kwargs, messages),
        )
