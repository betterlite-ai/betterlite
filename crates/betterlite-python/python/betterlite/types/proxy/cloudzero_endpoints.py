from datetime import datetime

from pydantic import BaseModel, Field, JsonValue


class CloudZeroInitRequest(BaseModel):
    api_key: str = Field(..., description="CloudZero API key for authentication")
    connection_id: str = Field(..., description="CloudZero connection ID for data submission")
    timezone: str = Field(default="UTC", description="Timezone for date handling (default: UTC)")


class CloudZeroInitResponse(BaseModel):
    message: str
    status: str


class CloudZeroExportRequest(BaseModel):
    limit: int | None = Field(None, description="Optional limit on number of records to export")
    operation: str = Field(
        default="replace_hourly",
        description="CloudZero operation type (replace_hourly or sum)",
    )
    start_time_utc: datetime | None = Field(None, description="Start time for data export in UTC")
    end_time_utc: datetime | None = Field(None, description="End time for data export in UTC")


class CloudZeroExportResponse(BaseModel):
    message: str
    status: str
    records_exported: int | None = None
    dry_run_data: dict[str, JsonValue] | None = Field(
        None, description="Dry run data including usage data and CBF transformed data"
    )
    summary: dict[str, JsonValue] | None = Field(None, description="Summary statistics for dry run")


class CloudZeroSettingsView(BaseModel):
    api_key_masked: str | None = Field(
        None, description="Masked API key showing only first 4 and last 4 characters"
    )
    connection_id: str | None = Field(
        None, description="CloudZero connection ID for data submission"
    )
    timezone: str | None = Field(None, description="Timezone for date handling")
    status: str | None = Field(None, description="Configuration status")


class CloudZeroSettingsUpdate(BaseModel):
    api_key: str | None = Field(None, description="New CloudZero API key for authentication")
    connection_id: str | None = Field(
        None, description="New CloudZero connection ID for data submission"
    )
    timezone: str | None = Field(None, description="New timezone for date handling")
