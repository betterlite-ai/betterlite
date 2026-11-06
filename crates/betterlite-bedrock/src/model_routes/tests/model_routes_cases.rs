#![cfg(test)]

use indexmap::{IndexMap, IndexSet};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::model_routes::catalog::{
    RESPONSE_FORMAT_FLAG, TOOLS_REASONING_FLAG, chat_completions_is_default, openai_gpt_version,
    price_map_flag,
};
use crate::model_routes::model_info::{BedrockModelInfo, ModelInfo as _};
use crate::model_routes::request::{
    request_needs_converse, response_format_needs_converse as format_needs_converse,
};
use crate::model_routes::{
    BedrockRoute, CHAT_COMPLETIONS_PREFIX, CLAUDE_PLATFORM_PREFIX, CONVERSE_ONLY_REQUEST_KEYS,
    CONVERSE_PREFIX, MANTLE_PREFIX, ModelRouteSelector, ParamMap, ParamValue, RouteError,
    get_bedrock_tool_name, get_claude_platform_model, has_route_prefix, map_claude_platform_auth,
    without_bedrock_route_prefix,
};

#[rstest]
#[case::explicit_route_prefixes(explicit_route_prefixes)]
#[case::gpt_runtime_default_capability(gpt_runtime_default_capability)]
#[case::converse_only_request_fields(converse_only_request_fields)]
#[case::response_format_needs_converse(response_format_needs_converse)]
#[case::tools_with_reasoning(tools_with_reasoning)]
#[case::additional_drop_before_route(additional_drop_before_route)]
#[case::catalog_and_unknown_route(catalog_and_unknown_route)]
#[case::route_helpers_and_workspace_keys(route_helpers_and_workspace_keys)]
#[case::model_info_passthrough(model_info_passthrough)]
#[case::reverse_tool_name_lookup(reverse_tool_name_lookup)]
fn model_routes_cases(#[case] check: fn()) {
    check();
}

fn text(value: &str) -> ParamValue {
    ParamValue::Text(value.to_owned())
}

fn number(value: i32) -> ParamValue {
    ParamValue::Number(serde_json::Number::from(value))
}

fn list(values: &[&str]) -> ParamValue {
    ParamValue::List(values.iter().map(|value| text(value)).collect())
}

fn mapping(pairs: &[(&str, ParamValue)]) -> ParamValue {
    ParamValue::Mapping(
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), value.clone()))
            .collect(),
    )
}

fn params(pairs: &[(&str, ParamValue)]) -> ParamMap {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn models(names: &[&str]) -> IndexSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn endpoint_entry(endpoint: &str) -> ParamValue {
    mapping(&[("supported_endpoints", list(&[endpoint]))])
}

fn flag_entry(flag: &str, value: ParamValue) -> ParamValue {
    mapping(&[(flag, value)])
}

fn selector<'catalog>(
    price_map: &'catalog ParamMap,
    converse: &'catalog IndexSet<String>,
    owned: bool,
) -> ModelRouteSelector<'catalog> {
    ModelRouteSelector::new(price_map, converse, owned).unwrap()
}

fn explicit_route_table(route: &ModelRouteSelector<'_>, fallback: &ParamMap) {
    let table = [
        ("invoke/anthropic.claude-3", BedrockRoute::Invoke),
        ("bedrock/invoke/anthropic.claude-3", BedrockRoute::Invoke),
        ("converse/invoke/x", BedrockRoute::Invoke),
        (
            "claude_platform/openai.gpt-5.6",
            BedrockRoute::ClaudePlatform,
        ),
        ("converse_like/x", BedrockRoute::ConverseLike),
        ("converse/x", BedrockRoute::Converse),
        ("agent/mantle/x", BedrockRoute::Agent),
        ("agentcore/x", BedrockRoute::Agentcore),
        ("async_invoke/x", BedrockRoute::AsyncInvoke),
        ("openai/gpt-5.6", BedrockRoute::Openai),
        ("mantle/openai.gpt-5.5", BedrockRoute::Mantle),
        ("bedrock/mantle/x", BedrockRoute::Mantle),
        ("bedrock_mantle/openai.gpt-5.5", BedrockRoute::Converse),
        ("xconverse/y", BedrockRoute::Invoke),
    ];
    for (model, expected) in table {
        assert_eq!(
            route.route_for_request(model, fallback, None).unwrap(),
            expected,
            "{model}"
        );
    }
}

fn explicit_route_prefixes() {
    let price_map = ParamMap::new();
    let converse = IndexSet::new();
    let route = selector(&price_map, &converse, false);
    let fallback = params(&[
        ("serviceTier", text("default")),
        ("stop", ParamValue::List(Vec::new())),
    ]);
    explicit_route_table(&route, &fallback);
    assert_eq!(
        route
            .route_for_request("chat_completions/x", &ParamMap::new(), None)
            .unwrap(),
        BedrockRoute::ChatCompletions
    );
    assert_eq!(
        route
            .route_for_request("chat_completions/x", &fallback, None)
            .unwrap(),
        BedrockRoute::Converse
    );
    assert_eq!(
        route
            .route_for_request("invoke/x", &fallback, None)
            .unwrap(),
        BedrockRoute::Invoke
    );
}

fn gpt_capability_map() -> ParamMap {
    params(&[
        ("openai.gpt-5.6", endpoint_entry("/v1/chat/completions")),
        ("openai.gpt-5.5", endpoint_entry("/v1/chat/completions")),
        (
            "us.openai.gpt-5.6-luna",
            endpoint_entry("/v1/chat/completions"),
        ),
        (
            "openai.gpt-oss-20b-1:0",
            flag_entry(TOOLS_REASONING_FLAG, ParamValue::Bool(true)),
        ),
        (
            "us-east-1/openai.gpt-5.6-sol",
            endpoint_entry("/v1/chat/completions"),
        ),
        (
            "openai.gpt-5.6-region",
            endpoint_entry("/v1/chat/completions"),
        ),
        ("openai.gpt-5.6-other", endpoint_entry("/v1/responses")),
        (
            "openai.gpt-5.6-string",
            flag_entry(RESPONSE_FORMAT_FLAG, text("true")),
        ),
        (
            "openai.gpt-5.6-number",
            flag_entry(TOOLS_REASONING_FLAG, number(1)),
        ),
        (
            "openai.gpt-5.6-true",
            flag_entry(RESPONSE_FORMAT_FLAG, ParamValue::Bool(true)),
        ),
        ("openai.gpt-5.6-null", ParamValue::Null),
    ])
}

fn check_gpt_versions(route: &ModelRouteSelector<'_>) {
    let versions = [
        ("openai.gpt-5.6", Some((5, 6))),
        ("us.openai.gpt-6.1-sol", Some((6, 1))),
        ("openai.gpt-6-astra", Some((6, 0))),
        ("openai.gpt-123", Some((123, 0))),
        ("openai.gpt-1234", None),
        ("openai.gpt-5.1234", Some((5, 0))),
        ("openai.gpt-5.6.7", Some((5, 6))),
        ("openai.gpt-0.5", Some((0, 5))),
        ("openai.gpt-oss-20b-1:0", None),
        ("xopenai.gpt-5.6", None),
        ("x.openai.gpt-5.6", Some((5, 6))),
    ];
    for (model, expected) in versions {
        assert_eq!(
            openai_gpt_version(route, model).unwrap(),
            expected,
            "{model}"
        );
    }
}

fn check_gpt_defaults(route: &ModelRouteSelector<'_>) {
    let defaults = [
        ("openai.gpt-5.6", true),
        ("openai.gpt-5.5", false),
        ("openai.gpt-oss-20b-1:0", false),
        ("us.openai.gpt-5.6-luna", true),
        ("bedrock/us-east-1/openai.gpt-5.6-sol", true),
        ("bedrock/us-east-1/openai.gpt-5.6-region", true),
        ("global.openai.gpt-5.6-other", false),
        ("bedrock/us-east-1/openai.gpt-4o", false),
        ("openai.gpt-5.6-null", false),
    ];
    for (model, expected) in defaults {
        assert_eq!(
            chat_completions_is_default(route, model).unwrap(),
            expected,
            "{model}"
        );
    }
}

fn check_gpt_flags(route: &ModelRouteSelector<'_>) {
    assert!(price_map_flag(route, "openai.gpt-5.6-true", RESPONSE_FORMAT_FLAG).unwrap());
    assert!(!price_map_flag(route, "openai.gpt-5.6-string", RESPONSE_FORMAT_FLAG).unwrap());
    assert!(!price_map_flag(route, "openai.gpt-5.6-number", TOOLS_REASONING_FLAG).unwrap());
    assert!(price_map_flag(route, "openai.gpt-oss-20b-1:0", TOOLS_REASONING_FLAG).unwrap());
}

fn gpt_runtime_default_capability() {
    let price_map = gpt_capability_map();
    let converse = IndexSet::new();
    let route = selector(&price_map, &converse, false);
    check_gpt_versions(&route);
    check_gpt_defaults(&route);
    check_gpt_flags(&route);
    let malformed = params(&[
        ("openai.gpt-5.6-bad", text("not an object")),
        (
            "bedrock/us-east-1/openai.gpt-5.6-dup",
            endpoint_entry("/v1/chat/completions"),
        ),
        ("openai.gpt-5.6-dup", list(&[])),
    ]);
    let broken = selector(&malformed, &converse, false);
    assert!(matches!(
        chat_completions_is_default(&broken, "openai.gpt-5.6-bad"),
        Err(RouteError::MalformedPriceEntry { .. })
    ));
    chat_completions_is_default(&broken, "bedrock/us-east-1/openai.gpt-5.6-dup").unwrap_err();
}

fn check_converse_only_key(route: &ModelRouteSelector<'_>, key: &str) {
    let values = [
        ParamValue::Bool(false),
        text(""),
        ParamValue::List(Vec::new()),
        ParamValue::Mapping(IndexMap::new()),
        number(0),
        text("set"),
    ];
    for value in &values {
        let request = params(&[(key, value.clone())]);
        assert_eq!(
            route
                .route_for_request("chat_completions/openai.gpt-4o", &request, None)
                .unwrap(),
            BedrockRoute::Converse,
            "{key}={value:?}"
        );
        assert_eq!(
            route
                .route_for_request("openai.gpt-5.6", &request, None)
                .unwrap(),
            BedrockRoute::Converse,
            "{key}={value:?}"
        );
    }
    let absent = params(&[("temperature", text("0.2"))]);
    assert_eq!(
        route
            .route_for_request("chat_completions/openai.gpt-4o", &absent, None)
            .unwrap(),
        BedrockRoute::ChatCompletions
    );
    let null = params(&[(key, ParamValue::Null)]);
    assert_eq!(
        route
            .route_for_request("openai.gpt-5.6", &null, None)
            .unwrap(),
        BedrockRoute::ChatCompletions,
        "{key} null"
    );
}

fn converse_only_request_fields() {
    let price_map = params(&[("openai.gpt-5.6", endpoint_entry("/v1/chat/completions"))]);
    let converse = IndexSet::new();
    let route = selector(&price_map, &converse, false);
    for key in CONVERSE_ONLY_REQUEST_KEYS {
        check_converse_only_key(&route, key);
    }
    assert_eq!(
        route.route("openai.gpt-5.6", None).unwrap(),
        BedrockRoute::ChatCompletions
    );
    assert_eq!(
        route.route("chat_completions/openai.gpt-4o", None).unwrap(),
        BedrockRoute::ChatCompletions
    );
    let owned = selector(&price_map, &converse, true);
    assert_eq!(
        owned
            .route("openai.gpt-5.6", Some(&ParamMap::new()))
            .unwrap(),
        BedrockRoute::Converse
    );
}

fn response_format_map() -> ParamMap {
    params(&[
        (
            "openai.gpt-5.6",
            flag_entry(RESPONSE_FORMAT_FLAG, ParamValue::Bool(true)),
        ),
        (
            "openai.gpt-oss-20b-1:0",
            endpoint_entry("/v1/chat/completions"),
        ),
    ])
}

fn check_native_formats(route: &ModelRouteSelector<'_>) {
    let native = [
        None,
        Some(mapping(&[("type", text("text"))])),
        Some(mapping(&[
            ("type", text("text")),
            ("json_schema", mapping(&[])),
        ])),
        Some(mapping(&[
            ("type", text("json_schema")),
            ("json_schema", ParamValue::Null),
        ])),
        Some(mapping(&[
            ("type", text("json_schema")),
            ("json_schema", mapping(&[("name", text("x"))])),
        ])),
    ];
    for format in &native {
        assert!(
            !format_needs_converse(route, "openai.gpt-5.6", format.as_ref()).unwrap(),
            "{format:?}"
        );
    }
}

fn check_converse_formats(route: &ModelRouteSelector<'_>) {
    let formats = [
        mapping(&[("type", text("json_object"))]),
        mapping(&[("type", text("json_schema"))]),
        mapping(&[]),
        mapping(&[("type", number(5))]),
    ];
    for format in &formats {
        assert!(
            format_needs_converse(route, "openai.gpt-5.6", Some(format)).unwrap(),
            "{format:?}"
        );
        assert!(
            format_needs_converse(route, "openai.gpt-oss-20b-1:0", Some(format)).unwrap(),
            "{format:?}"
        );
    }
}

fn response_format_needs_converse() {
    let price_map = response_format_map();
    let converse = IndexSet::new();
    let route = selector(&price_map, &converse, false);
    check_native_formats(&route);
    check_converse_formats(&route);
    assert!(!format_needs_converse(&route, "openai.gpt-oss-20b-1:0", None).unwrap());
    assert!(
        !format_needs_converse(&route, "openai.gpt-oss-20b-1:0", Some(&text("json schema")))
            .unwrap()
    );
    assert!(format_needs_converse(&route, "openai.gpt-5.6", Some(&text("json schema"))).unwrap());
    assert!(format_needs_converse(&route, "openai.gpt-5.6", Some(&list(&[]))).unwrap());
    let null_request = params(&[("response_format", ParamValue::Null)]);
    assert!(!request_needs_converse(&route, "openai.gpt-oss-20b-1:0", &null_request).unwrap());
}

fn tools_map() -> ParamMap {
    params(&[
        ("openai.gpt-5.6", endpoint_entry("/v1/chat/completions")),
        (
            "openai.gpt-oss-20b-1:0",
            flag_entry(TOOLS_REASONING_FLAG, ParamValue::Bool(true)),
        ),
        (
            "openai.gpt-5.6-literal",
            flag_entry(TOOLS_REASONING_FLAG, text("true")),
        ),
        (
            "openai.gpt-5.6-number",
            flag_entry(TOOLS_REASONING_FLAG, number(1)),
        ),
    ])
}

fn check_empty_tools(route: &ModelRouteSelector<'_>) {
    let empty_tools = params(&[("tools", ParamValue::List(Vec::new()))]);
    let empty_functions = params(&[("functions", ParamValue::List(Vec::new()))]);
    assert!(!request_needs_converse(route, "openai.gpt-5.6", &empty_tools).unwrap());
    assert!(!request_needs_converse(route, "openai.gpt-5.6", &empty_functions).unwrap());
}

fn check_tool_fallbacks(route: &ModelRouteSelector<'_>) {
    let with_tools = params(&[("tools", list(&["tool"]))]);
    let with_functions = params(&[("functions", list(&["function"]))]);
    assert!(!request_needs_converse(route, "openai.gpt-oss-20b-1:0", &with_tools).unwrap());
    assert!(!request_needs_converse(route, "openai.gpt-oss-20b-1:0", &with_functions).unwrap());
    let off_effort = params(&[
        ("tools", list(&["tool"])),
        ("reasoning_effort", text("none")),
    ]);
    assert!(!request_needs_converse(route, "openai.gpt-5.6", &off_effort).unwrap());
    for effort in [text("low"), text("high"), ParamValue::Null] {
        let request = params(&[("tools", list(&["tool"])), ("reasoning_effort", effort)]);
        assert!(request_needs_converse(route, "openai.gpt-5.6", &request).unwrap());
    }
    assert!(request_needs_converse(route, "openai.gpt-5.6", &with_tools).unwrap());
    assert!(!request_needs_converse(route, "openai.gpt-5.6-literal", &off_effort).unwrap());
    assert!(request_needs_converse(route, "openai.gpt-5.6-literal", &with_tools).unwrap());
    assert!(request_needs_converse(route, "openai.gpt-5.6-number", &with_tools).unwrap());
}

fn check_wrong_types(route: &ModelRouteSelector<'_>) {
    let wrong_tools = params(&[("tools", mapping(&[("tool", text("x"))]))]);
    assert!(matches!(
        request_needs_converse(route, "openai.gpt-5.6", &wrong_tools),
        Err(RouteError::WrongType { field: "tools" })
    ));
    let wrong_effort = params(&[("tools", list(&["tool"])), ("reasoning_effort", number(2))]);
    assert!(matches!(
        request_needs_converse(route, "openai.gpt-5.6", &wrong_effort),
        Err(RouteError::WrongType {
            field: "reasoning_effort"
        })
    ));
}

fn tools_with_reasoning() {
    let price_map = tools_map();
    let converse = IndexSet::new();
    let route = selector(&price_map, &converse, false);
    check_empty_tools(&route);
    check_tool_fallbacks(&route);
    check_wrong_types(&route);
}

fn additional_drop_before_route() {
    let price_map = ParamMap::new();
    let converse = IndexSet::new();
    let route = selector(&price_map, &converse, false);
    let request = params(&[
        ("stop", ParamValue::List(Vec::new())),
        ("model_id", text("profile-arn")),
        ("tools", list(&["tool"])),
        ("reasoning_effort", text("high")),
        ("temperature", number(0)),
        ("model", text("chat_completions/openai.gpt-4o")),
    ]);
    let original = request.clone();
    let model = "chat_completions/openai.gpt-4o";
    assert_eq!(
        route.route_for_request(model, &request, None).unwrap(),
        BedrockRoute::Converse
    );
    let dropped = [
        "stop".to_owned(),
        "model_id".to_owned(),
        "tools".to_owned(),
        "reasoning_effort".to_owned(),
    ];
    assert_eq!(
        route
            .route_for_request(model, &request, Some(&dropped))
            .unwrap(),
        BedrockRoute::ChatCompletions
    );
    assert_eq!(request, original);
    drop_list_edges(&route, model, &request);
}

fn drop_list_edges(route: &ModelRouteSelector<'_>, model: &str, request: &ParamMap) {
    assert_eq!(
        route.route_for_request(model, request, None).unwrap(),
        BedrockRoute::Converse
    );
    assert_eq!(
        route.route_for_request(model, request, Some(&[])).unwrap(),
        BedrockRoute::Converse
    );
    let missing = ["absent".to_owned()];
    assert_eq!(
        route
            .route_for_request(model, request, Some(&missing))
            .unwrap(),
        BedrockRoute::Converse
    );
    let dropped: Vec<String> = ["stop", "model_id", "tools", "reasoning_effort"]
        .iter()
        .map(|key| (*key).to_owned())
        .collect();
    let first = route
        .route_for_request(model, request, Some(&dropped))
        .unwrap();
    let second = route
        .route_for_request(model, request, Some(&dropped))
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(
        route
            .route_for_request(model, &params(&[("stop", ParamValue::Null)]), None)
            .unwrap(),
        BedrockRoute::ChatCompletions
    );
}

fn catalog_route_table() -> [(&'static str, BedrockRoute); 8] {
    [
        ("bedrock/amazon.nova-lite-v1:0", BedrockRoute::Converse),
        ("amazon.nova-lite-v1:0", BedrockRoute::Converse),
        ("nova-2/custom", BedrockRoute::Converse),
        ("nova/custom", BedrockRoute::Converse),
        ("bedrock/nova/custom", BedrockRoute::Converse),
        (
            "arn:aws:bedrock:us-east-1:1:application-inference-profile/abc",
            BedrockRoute::Converse,
        ),
        ("openai.gpt-5.6-sol", BedrockRoute::ChatCompletions),
        (
            "bedrock/us-east-1/openai.gpt-5.6-region",
            BedrockRoute::ChatCompletions,
        ),
    ]
}

fn catalog_unknown_table() -> [(&'static str, BedrockRoute); 8] {
    [
        (
            "anthropic.claude-3-haiku-20240307-v1:0",
            BedrockRoute::Converse,
        ),
        (
            "bedrock/us-east-1/anthropic.claude-3-haiku-20240307-v1:0",
            BedrockRoute::Converse,
        ),
        ("qwen.qwen3-32b-v1:0", BedrockRoute::Converse),
        ("openai.gpt-4o", BedrockRoute::Converse),
        ("global.openai.gpt-5.6-other", BedrockRoute::Converse),
        ("xopenai.gpt-4o", BedrockRoute::Invoke),
        ("meta.llama3-8b-instruct-v1:0", BedrockRoute::Invoke),
        ("bedrock_mantle/openai.gpt-5.5", BedrockRoute::Converse),
    ]
}

fn check_routes(route: &ModelRouteSelector<'_>, table: &[(&str, BedrockRoute)]) {
    for (model, expected) in table {
        assert_eq!(route.route(model, None).unwrap(), *expected, "{model}");
    }
}

fn catalog_and_unknown_route() {
    let price_map = params(&[
        ("openai.gpt-5.6-sol", endpoint_entry("/v1/chat/completions")),
        (
            "openai.gpt-5.6-region",
            endpoint_entry("/v1/chat/completions"),
        ),
    ]);
    let converse = models(&[
        "amazon.nova-lite-v1:0",
        "anthropic.claude-3-haiku-20240307-v1:0",
        "qwen.qwen3-32b-v1:0",
    ]);
    let route = selector(&price_map, &converse, false);
    check_routes(&route, &catalog_route_table());
    check_routes(&route, &catalog_unknown_table());
}

fn without_prefix_cases() {
    let stripped = [
        ("converse/chat_completions/x", "x"),
        ("chat_completions/converse/x", "x"),
        ("converse/a/converse/b", "a/b"),
        ("chat_completions/a/chat_completions/b", "a/b"),
        ("plain", "plain"),
        ("xconverse/y", "xy"),
    ];
    for (model, expected) in stripped {
        assert_eq!(without_bedrock_route_prefix(model), expected, "{model}");
    }
}

fn platform_model_cases() {
    let platform = [
        (
            "bedrock/claude_platform/x/claude_platform/y",
            "bedrock/x/claude_platform/y",
        ),
        ("claude_platform/x", "x"),
        ("plain", "plain"),
        ("xclaude_platform/y", "xy"),
    ];
    for (model, expected) in platform {
        assert_eq!(get_claude_platform_model(model), expected, "{model}");
    }
}

fn prefix_predicate_cases() {
    let prefixes = [
        (CONVERSE_PREFIX, "converse/x", true),
        (CONVERSE_PREFIX, "bedrock/converse/x", true),
        (CONVERSE_PREFIX, "a/b/converse/x", true),
        (CONVERSE_PREFIX, "xconverse/x", false),
        (CONVERSE_PREFIX, "converse_like/x", false),
        (CLAUDE_PLATFORM_PREFIX, "converse/claude_platform/x", true),
        (CLAUDE_PLATFORM_PREFIX, "xclaude_platform/x", false),
        (MANTLE_PREFIX, "mantle/x", true),
        (MANTLE_PREFIX, "x/mantle/y", true),
        (MANTLE_PREFIX, "bedrock_mantle/x", false),
        (CHAT_COMPLETIONS_PREFIX, "bedrock/chat_completions/x", true),
        (CHAT_COMPLETIONS_PREFIX, "xchat_completions/x", false),
    ];
    for (prefix, model, expected) in prefixes {
        assert_eq!(
            has_route_prefix(model, prefix),
            expected,
            "{prefix} {model}"
        );
    }
}

fn route_prefix_helpers() {
    without_prefix_cases();
    platform_model_cases();
    prefix_predicate_cases();
}

fn workspace_keys() {
    let passed = params(&[
        ("workspace_id", text("w")),
        ("aws_workspace_id", ParamValue::Null),
        ("anthropic_workspace_id", text("")),
    ]);
    let mut optional = params(&[("workspace_id", text("old")), ("keep", text("1"))]);
    map_claude_platform_auth(&passed, &mut optional);
    assert_eq!(
        optional,
        params(&[
            ("workspace_id", text("w")),
            ("keep", text("1")),
            ("aws_workspace_id", ParamValue::Null),
            ("anthropic_workspace_id", text("")),
        ])
    );
    let mut untouched = params(&[("keep", text("1"))]);
    map_claude_platform_auth(&ParamMap::new(), &mut untouched);
    assert_eq!(untouched, params(&[("keep", text("1"))]));
}

fn route_helpers_and_workspace_keys() {
    route_prefix_helpers();
    workspace_keys();
}

fn model_info_passthrough() {
    assert_eq!(BedrockModelInfo::get_api_base(None), None);
    assert_eq!(
        BedrockModelInfo::get_api_base(Some(String::new())),
        Some(String::new())
    );
    assert_eq!(
        BedrockModelInfo::get_api_base(Some("https://base.example".to_owned())),
        Some("https://base.example".to_owned())
    );
    assert!(BedrockModelInfo::get_api_key(None).is_none());
    let empty = BedrockModelInfo::get_api_key(Some(SecretString::from("")));
    assert_eq!(
        empty.map(|value| value.expose_secret().to_owned()),
        Some(String::new())
    );
    let supplied = BedrockModelInfo::get_api_key(Some(SecretString::from("sk-fixture")));
    assert_eq!(
        supplied.map(|value| value.expose_secret().to_owned()),
        Some("sk-fixture".to_owned())
    );
    let headers: IndexMap<String, String> = IndexMap::from([("x-amz".to_owned(), String::new())]);
    assert_eq!(
        BedrockModelInfo::validate_environment(headers.clone()),
        headers
    );
    assert!(BedrockModelInfo::get_models().is_empty());
}

fn reverse_tool_name_lookup() {
    let mappings = IndexMap::from([
        ("converted".to_owned(), "Original Name".to_owned()),
        ("x".to_owned(), "y".to_owned()),
        ("same".to_owned(), "same".to_owned()),
    ]);
    assert_eq!(
        get_bedrock_tool_name(&mappings, "converted"),
        "Original Name"
    );
    assert_eq!(get_bedrock_tool_name(&mappings, "missing"), "missing");
    assert_eq!(get_bedrock_tool_name(&mappings, ""), "");
    assert_eq!(get_bedrock_tool_name(&mappings, "same"), "same");
    assert_eq!(
        get_bedrock_tool_name(&mappings, "Original Name"),
        "Original Name"
    );
}
