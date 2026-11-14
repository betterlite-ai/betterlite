from typing import Literal

from pydantic import BaseModel, Field, JsonValue, RootModel

from betterlite.types.interactions.content import Content
from betterlite.types.interactions.deltas import ContentDelta
from betterlite.types.interactions.enums import Status1
from betterlite.types.interactions.interaction import Interaction


class Error(BaseModel):
    code: str | None = None
    message: str | None = None


class InteractionStatusUpdate(BaseModel):
    interaction_id: str | None = None
    status: Status1 | None = None
    event_type: Literal["interaction.status_update"] = "interaction.status_update"
    event_id: str | None = None


class ContentStart(BaseModel):
    index: int | None = None
    content: Content | None = None
    event_type: Literal["content.start"] = "content.start"
    event_id: str | None = None


class ContentStop(BaseModel):
    index: int | None = None
    event_type: Literal["content.stop"] = "content.stop"
    event_id: str | None = None


class ErrorEvent(BaseModel):
    event_type: Literal["error"] = "error"
    error: Error | None = None
    event_id: str | None = None


class InteractionEvent(BaseModel):
    event_type: Literal["interaction.start", "interaction.complete"]
    interaction: Interaction | None = None
    event_id: str | None = None


class StepStart(BaseModel):
    event_type: Literal["step.start"] = "step.start"
    index: int | None = None
    step: dict[str, JsonValue] | None = None
    event_id: str | None = None


class StepDelta(BaseModel):
    event_type: Literal["step.delta"] = "step.delta"
    index: int | None = None
    delta: dict[str, JsonValue] | None = None
    event_id: str | None = None


class StepStop(BaseModel):
    event_type: Literal["step.stop"] = "step.stop"
    index: int | None = None
    status: str | None = None
    event_id: str | None = None


class InteractionCreated(BaseModel):
    event_type: Literal["interaction.created"] = "interaction.created"
    interaction: dict[str, JsonValue] | None = None
    event_id: str | None = None


class InteractionInProgress(BaseModel):
    event_type: Literal["interaction.in_progress"] = "interaction.in_progress"
    interaction_id: str | None = None
    event_id: str | None = None


class InteractionCompleted(BaseModel):
    event_type: Literal["interaction.completed"] = "interaction.completed"
    interaction: dict[str, JsonValue] | None = None
    event_id: str | None = None


class InteractionRequiresAction(BaseModel):
    event_type: Literal["interaction.requires_action"] = "interaction.requires_action"
    interaction_id: str | None = None
    event_id: str | None = None


class InteractionSseEvent(
    RootModel[
        StepStart
        | StepDelta
        | StepStop
        | InteractionCreated
        | InteractionInProgress
        | InteractionCompleted
        | InteractionRequiresAction
        | InteractionEvent
        | InteractionStatusUpdate
        | ContentStart
        | ContentDelta
        | ContentStop
        | ErrorEvent
    ]
):
    root: (
        StepStart
        | StepDelta
        | StepStop
        | InteractionCreated
        | InteractionInProgress
        | InteractionCompleted
        | InteractionRequiresAction
        | InteractionEvent
        | InteractionStatusUpdate
        | ContentStart
        | ContentDelta
        | ContentStop
        | ErrorEvent
    ) = Field(discriminator="event_type")
