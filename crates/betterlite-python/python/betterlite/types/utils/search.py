from enum import Enum


class SearchProviders(str, Enum):
    PERPLEXITY = "perplexity"
    TAVILY = "tavily"
    PARALLEL_AI = "parallel_ai"
    EXA_AI = "exa_ai"
    BRAVE = "brave"
    GOOGLE_PSE = "google_pse"
    DATAFORSEO = "dataforseo"
    FIRECRAWL = "firecrawl"
    FASTCRW = "fastcrw"
    SEARXNG = "searxng"
    LINKUP = "linkup"
    DUCKDUCKGO = "duckduckgo"
    SEARCHAPI = "searchapi"
    SERPER = "serper"
    YOU_COM = "you_com"
    APISERPENT = "apiserpent"
    TINYFISH = "tinyfish"
    AGENTCORE = "agentcore"
    NIMBLE = "nimble"
    BING_GROUNDING = "bing_grounding"


SearchProvider = SearchProviders

__all__ = ["SearchProvider", "SearchProviders"]
