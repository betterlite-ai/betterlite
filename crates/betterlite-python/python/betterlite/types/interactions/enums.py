from enum import Enum


class Language(Enum):
    python = "python"


class Status(Enum):
    success = "success"
    error = "error"
    paywall = "paywall"
    unsafe = "unsafe"


class Environment(Enum):
    browser = "browser"


class EventType(Enum):
    interaction_start = "interaction.start"
    interaction_complete = "interaction.complete"


class Status1(Enum):
    in_progress = "in_progress"
    requires_action = "requires_action"
    completed = "completed"
    failed = "failed"
    cancelled = "cancelled"
    incomplete = "incomplete"
    budget_exceeded = "budget_exceeded"
    queued = "queued"


class MediaResolution(Enum):
    low = "low"
    medium = "medium"
    high = "high"


class ToolChoiceType(Enum):
    auto = "auto"
    any = "any"
    none = "none"
    validated = "validated"


class ThinkingLevel(Enum):
    low = "low"
    high = "high"


class ThinkingSummaries(Enum):
    auto = "auto"
    none = "none"


class ResponseModality(Enum):
    text = "text"
    image = "image"
    audio = "audio"


class Status3(Enum):
    IN_PROGRESS = "in_progress"
    REQUIRES_ACTION = "requires_action"
    COMPLETED = "completed"
    FAILED = "failed"
    CANCELLED = "cancelled"
    INCOMPLETE = "incomplete"
    BUDGET_EXCEEDED = "budget_exceeded"
    QUEUED = "queued"
