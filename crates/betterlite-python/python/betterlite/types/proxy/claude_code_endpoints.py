from pydantic import BaseModel, Field


class PluginAuthor(BaseModel):
    name: str = Field(..., description="Author name")
    email: str | None = Field(None, description="Author email")


class PluginOwner(BaseModel):
    name: str = Field(..., description="Owner name")
    email: str | None = Field(None, description="Owner email")


class PluginSpec(BaseModel):
    source: dict[str, str] = Field(
        ...,
        description=(
            "Plugin source reference. Supported formats:\n"
            "- GitHub: {'source': 'github', 'repo': 'org/repo'}\n"
            "- Git URL: {'source': 'url', 'url': 'https://github.com/org/repo.git'}\n"
            "- Git Subdir: {'source': 'git-subdir', 'url': 'https://github.com/org/repo.git', "
            "'path': 'plugins/plugin-name'}\n"
            "- Zip archive on any https host (e.g. S3): {'source': 'archive', "
            "'url': 'https://bucket.s3.amazonaws.com/plugin.zip', "
            "'sha256': '<optional hex digest>'}"
        ),
    )
    version: str | None = Field("1.0.0", description="Semantic version")
    description: str | None = Field(None, description="Plugin description")
    author: PluginAuthor | None = Field(None, description="Plugin author")
    homepage: str | None = Field(None, description="Plugin homepage URL")
    keywords: list[str] | None = Field(None, description="Search keywords")
    category: str | None = Field(None, description="Plugin category")
    domain: str | None = Field(None, description="Skill domain (e.g., 'Productivity')")
    namespace: str | None = Field(
        None, description="Skill namespace within domain (e.g., 'workflows')"
    )


class RegisterPluginRequest(PluginSpec):
    name: str = Field(
        ...,
        description="Plugin name (kebab-case, e.g., 'my-plugin')",
        pattern=r"^[a-z0-9-]+$",
    )


class UpdatePluginRequest(PluginSpec):
    version: str | None = Field(None, description="Semantic version; cleared if omitted")


class PluginResponse(BaseModel):
    id: str = Field(..., description="Plugin unique ID")
    name: str = Field(..., description="Plugin name")
    version: str | None = Field(None, description="Plugin version")
    description: str | None = Field(None, description="Plugin description")
    source: dict[str, str] = Field(..., description="Plugin source reference")
    enabled: bool = Field(..., description="Whether plugin is enabled")


class RegisterPluginResponse(BaseModel):
    status: str = Field(..., description="Operation status")
    action: str = Field(..., description="Action taken (created/updated)")
    plugin: PluginResponse = Field(..., description="Plugin information")


class PluginListItem(BaseModel):
    id: str
    name: str
    version: str | None
    description: str | None
    source: dict[str, str]
    author: PluginAuthor | None = None
    homepage: str | None = None
    keywords: list[str] | None = None
    category: str | None = None
    domain: str | None = None
    namespace: str | None = None
    enabled: bool
    created_at: str | None
    updated_at: str | None


class ListPluginsResponse(BaseModel):
    plugins: list[PluginListItem]
    count: int


class MarketplacePluginEntry(BaseModel):
    name: str
    source: dict[str, str]
    version: str | None = None
    description: str | None = None
    author: PluginAuthor | None = None
    homepage: str | None = None
    keywords: list[str] | None = None
    category: str | None = None


class MarketplaceResponse(BaseModel):
    name: str = Field(..., description="Marketplace identifier")
    owner: PluginOwner = Field(..., description="Marketplace owner")
    plugins: list[MarketplacePluginEntry] = Field(
        default_factory=list, description="Available plugins"
    )
