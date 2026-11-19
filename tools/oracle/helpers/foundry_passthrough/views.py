from pydantic import BaseModel, ConfigDict, JsonValue


class PageView(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    index: int
    markdown: str


class UsageView(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    pages_processed: int | None = None
    credits: float | None = None
    doc_size_bytes: int | None = None


class OcrView(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    pages: list[PageView]
    model: str
    usage_info: UsageView | None = None


class StandardView(BaseModel):
    response: JsonValue


class LoggingView(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    call_type: str
