use crate::rerank::url::complete_url;

const BASE: &str = "https://res.services.ai.azure.com";
const MISSING: &str =
    "Azure AI API Base is required. api_base=None. Set in call or via `AZURE_AI_API_BASE` env var.";
const RELATIVE: &str = "Azure AI API Base must be an absolute URL including scheme (e.g. 'https://<resource>.services.ai.azure.com'). Got api_base=";

// the trailing path of the base and the trailing path LiteLLM answers, checked against httpx 0.28.1
const PATHS: [(&str, &str); 25] = [
    ("", "/v1/rerank"),
    ("/", "/v1/rerank"),
    ("/models/", "/models/v1/rerank"),
    ("/v1/rerank", "/v1/rerank"),
    ("/v1/rerank/", "/v1/rerank"),
    ("/v1/rerank///", "/v1/rerank"),
    ("/v2/rerank", "/v2/rerank"),
    ("/providers/cohere/v2/rerank", "/providers/cohere/v2/rerank"),
    (
        "/providers/cohere/v2/rerank/",
        "/providers/cohere/v2/rerank",
    ),
    ("/v1", "/v1/rerank"),
    ("/v2", "/v2/rerank"),
    ("/providers/cohere/v2", "/providers/cohere/v2/rerank"),
    ("/providers/cohere/v2/", "/providers/cohere/v2/rerank"),
    (
        "/v1?api-version=2024-05-01",
        "/v1/rerank?api-version=2024-05-01",
    ),
    (
        "/v1/rerank?api-version=2024-05-01&x=1#frag",
        "/v1/rerank?api-version=2024-05-01&x=1#frag",
    ),
    ("/prefix?x=a%20b&x=2#f", "/prefix/v1/rerank?x=a+b&x=2#f"),
    ("/V1", "/V1/v1/rerank"),
    ("/openai/v1", "/openai/v1/rerank"),
    ("/v1/rerankX", "/v1/rerankX/v1/rerank"),
    ("//v1/rerank//", "//v1/rerank"),
    ("/v2?api-version=1#f", "/v2/rerank?api-version=1#f"),
    ("/a b", "/a%20b/v1/rerank"),
    ("/v1/rer%2Fank", "/v1/rer/ank/v1/rerank"),
    ("/a%41b", "/aAb/v1/rerank"),
    ("/it's%20here", "/it's%20here/v1/rerank"),
];

pub(super) fn rerank_url_api_version() {
    assert_eq!(complete_url(None).unwrap_err().to_string(), MISSING);
    assert_relative("/v1/rerank", "'/v1/rerank'");
    assert_relative("res.services.ai.azure.com", "'res.services.ai.azure.com'");
    assert_relative("https://", "'https://'");
    assert_relative("/it's", "\"/it's\"");
    assert_relative("/both'\"kinds", "'/both\\'\"kinds'");
    for (path, expected) in PATHS {
        let base = format!("{BASE}{path}");
        assert_eq!(
            complete_url(Some(&base)).unwrap(),
            format!("{BASE}{expected}"),
            "{path}"
        );
    }
    assert_eq!(
        complete_url(Some("HTTPS://RES.Services.AI.Azure.com/V2/")).unwrap(),
        "https://res.services.ai.azure.com/V2/v1/rerank"
    );
}

fn assert_relative(base: &str, rendered: &str) {
    let error = complete_url(Some(base)).unwrap_err();
    assert_eq!(error.to_string(), format!("{RELATIVE}{rendered}."));
}
