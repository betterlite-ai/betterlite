#![cfg(test)]

use std::fmt::Debug;

use indexmap::{IndexMap, IndexSet};
use rstest::rstest;

use crate::model_routes::catalog::{
    RESPONSE_FORMAT_FLAG, TOOLS_REASONING_FLAG, chat_completions_is_default, openai_gpt_version,
    price_map_flag,
};
use crate::model_routes::request::{
    request_needs_converse, response_format_needs_converse as format_needs_converse,
};
use crate::model_routes::{
    BedrockRoute, CONVERSE_ONLY_REQUEST_KEYS, ModelRouteSelector, ParamMap, ParamValue, RouteError,
    get_claude_platform_model, map_claude_platform_auth, without_bedrock_route_prefix,
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

fn check_cases<T, F>(cases: &[(&str, T)], check: F)
where
    T: PartialEq + Debug,
    F: Fn(&str) -> T,
{
    for (key, expected) in cases {
        assert_eq!(&check(key), expected, "{key}");
    }
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

fn explicit_route_prefixes() {
    let price_map = ParamMap::new();
    let converse = IndexSet::new();
    let route = selector(&price_map, &converse, false);
    let fallback = params(&[
        ("serviceTier", text("default")),
        ("stop", ParamValue::List(Vec::new())),
    ]);
    check_cases(
        &[
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
        ],
        |model| route.route_for_request(model, &fallback, None).unwrap(),
    );
    chat_completions_prefix_cases(&route, &fallback);
}

fn chat_completions_prefix_cases(route: &ModelRouteSelector<'_>, fallback: &ParamMap) {
    assert_eq!(
        route
            .route_for_request("chat_completions/x", &ParamMap::new(), None)
            .unwrap(),
        BedrockRoute::ChatCompletions
    );
    assert_eq!(
        route
            .route_for_request("chat_completions/x", fallback, None)
            .unwrap(),
        BedrockRoute::Converse
    );
    assert_eq!(
        route.route_for_request("invoke/x", fallback, None).unwrap(),
        BedrockRoute::Invoke
    );
}

fn capability_map() -> ParamMap {
    params(&[
        (
            "openai.gpt-5.6",
            mapping(&[
                ("supported_endpoints", list(&["/v1/chat/completions"])),
                (RESPONSE_FORMAT_FLAG, ParamValue::Bool(true)),
            ]),
        ),
        ("openai.gpt-5.5", endpoint_entry("/v1/chat/completions")),
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
            "openai.gpt-5.6-text-endpoints",
            mapping(&[("supported_endpoints", text("/v1/chat/completions"))]),
        ),
        (
            "openai.gpt-5.6-string",
            mapping(&[
                (RESPONSE_FORMAT_FLAG, text("true")),
                (TOOLS_REASONING_FLAG, text("true")),
            ]),
        ),
        (
            "openai.gpt-5.6-number",
            flag_entry(TOOLS_REASONING_FLAG, number(1)),
        ),
        ("openai.gpt-5.6-null", ParamValue::Null),
    ])
}

fn gpt_version_checks(route: &ModelRouteSelector<'_>) {
    check_cases(
        &[
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
        ],
        |model| openai_gpt_version(route, model).unwrap(),
    );
    check_cases(
        &[
            ("openai.gpt-5.6", true),
            ("openai.gpt-5.5", false),
            ("openai.gpt-oss-20b-1:0", false),
            ("bedrock/us-east-1/openai.gpt-5.6-sol", true),
            ("bedrock/us-east-1/openai.gpt-5.6-region", true),
            ("global.openai.gpt-5.6-other", false),
            ("bedrock/us-east-1/openai.gpt-4o", false),
            ("openai.gpt-5.6-text-endpoints", false),
            ("openai.gpt-5.6-null", false),
        ],
        |model| chat_completions_is_default(route, model).unwrap(),
    );
}

fn gpt_flag_checks(route: &ModelRouteSelector<'_>) {
    assert!(price_map_flag(route, "openai.gpt-5.6", RESPONSE_FORMAT_FLAG).unwrap());
    assert!(!price_map_flag(route, "openai.gpt-5.6-string", RESPONSE_FORMAT_FLAG).unwrap());
    assert!(!price_map_flag(route, "openai.gpt-5.6-number", TOOLS_REASONING_FLAG).unwrap());
    assert!(price_map_flag(route, "openai.gpt-oss-20b-1:0", TOOLS_REASONING_FLAG).unwrap());
}

fn gpt_malformed_checks() {
    let malformed = params(&[
        ("openai.gpt-5.6-bad", text("not an object")),
        (
            "bedrock/us-east-1/openai.gpt-5.6-dup",
            endpoint_entry("/v1/chat/completions"),
        ),
        ("openai.gpt-5.6-dup", list(&[])),
    ]);
    let converse = IndexSet::new();
    let broken = selector(&malformed, &converse, false);
    assert!(matches!(
        chat_completions_is_default(&broken, "openai.gpt-5.6-bad"),
        Err(RouteError::MalformedPriceEntry { .. })
    ));
    chat_completions_is_default(&broken, "bedrock/us-east-1/openai.gpt-5.6-dup").unwrap_err();
}

fn gpt_runtime_default_capability() {
    let price_map = capability_map();
    let converse = IndexSet::new();
    let route = selector(&price_map, &converse, false);
    gpt_version_checks(&route);
    gpt_flag_checks(&route);
    gpt_malformed_checks();
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

fn format_outcomes(route: &ModelRouteSelector<'_>, model: &str) -> Vec<bool> {
    let formats = [
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
        Some(mapping(&[("type", text("json_object"))])),
        Some(mapping(&[("type", text("json_schema"))])),
        Some(mapping(&[])),
        Some(mapping(&[("type", number(5))])),
    ];
    let mut outcomes = Vec::new();
    for format in &formats {
        outcomes.push(format_needs_converse(route, model, format.as_ref()).unwrap());
    }
    outcomes
}

fn response_format_needs_converse() {
    let price_map = capability_map();
    let converse = IndexSet::new();
    let route = selector(&price_map, &converse, false);
    check_cases(
        &[
            (
                "openai.gpt-5.6",
                vec![false, false, false, false, false, true, true, true, true],
            ),
            (
                "openai.gpt-oss-20b-1:0",
                vec![false, false, false, true, true, true, true, true, true],
            ),
        ],
        |model| format_outcomes(&route, model),
    );
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

fn tool_requests() -> Vec<ParamMap> {
    [
        vec![("tools", ParamValue::List(Vec::new()))],
        vec![("functions", ParamValue::List(Vec::new()))],
        vec![
            ("tools", list(&["tool"])),
            ("reasoning_effort", text("none")),
        ],
        vec![("tools", list(&["tool"]))],
        vec![("functions", list(&["function"]))],
        vec![
            ("tools", list(&["tool"])),
            ("reasoning_effort", ParamValue::Null),
        ],
        vec![
            ("tools", list(&["tool"])),
            ("reasoning_effort", text("low")),
        ],
    ]
    .into_iter()
    .map(|pairs| params(&pairs))
    .collect()
}

fn tool_outcomes(route: &ModelRouteSelector<'_>, model: &str) -> Vec<bool> {
    let requests = tool_requests();
    let mut outcomes = Vec::new();
    for request in &requests {
        outcomes.push(request_needs_converse(route, model, request).unwrap());
    }
    outcomes
}

fn tools_with_reasoning() {
    let price_map = capability_map();
    let converse = IndexSet::new();
    let route = selector(&price_map, &converse, false);
    check_cases(
        &[
            (
                "openai.gpt-5.6",
                vec![false, false, false, true, true, true, true],
            ),
            (
                "openai.gpt-oss-20b-1:0",
                vec![false, false, false, false, false, false, false],
            ),
            (
                "openai.gpt-5.6-string",
                vec![false, false, false, true, true, true, true],
            ),
            (
                "openai.gpt-5.6-number",
                vec![false, false, false, true, true, true, true],
            ),
        ],
        |model| tool_outcomes(&route, model),
    );
    let wrong_tools = params(&[("tools", mapping(&[("tool", text("x"))]))]);
    assert!(matches!(
        request_needs_converse(&route, "openai.gpt-5.6", &wrong_tools),
        Err(RouteError::WrongType { field: "tools" })
    ));
    let wrong_effort = params(&[("tools", list(&["tool"])), ("reasoning_effort", number(2))]);
    assert!(matches!(
        request_needs_converse(&route, "openai.gpt-5.6", &wrong_effort),
        Err(RouteError::WrongType {
            field: "reasoning_effort"
        })
    ));
}

fn drop_request() -> ParamMap {
    params(&[
        ("stop", ParamValue::List(Vec::new())),
        ("model_id", text("profile-arn")),
        ("tools", list(&["tool"])),
        ("reasoning_effort", text("high")),
        ("temperature", number(0)),
        ("model", text("chat_completions/openai.gpt-4o")),
    ])
}

fn additional_drop_before_route() {
    let price_map = ParamMap::new();
    let converse = IndexSet::new();
    let route = selector(&price_map, &converse, false);
    let request = drop_request();
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

fn catalog_route_table() -> [(&'static str, BedrockRoute); 16] {
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
    check_cases(&catalog_route_table(), |model| {
        route.route(model, None).unwrap()
    });
}

fn model_rewrite_cases() {
    check_cases(
        &[
            ("converse/chat_completions/x", "x".to_owned()),
            ("chat_completions/converse/x", "x".to_owned()),
            ("converse/a/converse/b", "a/b".to_owned()),
            ("chat_completions/a/chat_completions/b", "a/b".to_owned()),
            ("plain", "plain".to_owned()),
            ("xconverse/y", "xy".to_owned()),
        ],
        without_bedrock_route_prefix,
    );
    check_cases(
        &[
            (
                "bedrock/claude_platform/x/claude_platform/y",
                "bedrock/x/claude_platform/y".to_owned(),
            ),
            ("claude_platform/x", "x".to_owned()),
            ("plain", "plain".to_owned()),
            ("xclaude_platform/y", "xy".to_owned()),
        ],
        get_claude_platform_model,
    );
}

fn prefix_predicate_cases() {
    let price_map = ParamMap::new();
    let converse = IndexSet::new();
    let route = selector(&price_map, &converse, false);
    check_cases(
        &[
            ("converse/x", BedrockRoute::Converse),
            ("bedrock/converse/x", BedrockRoute::Converse),
            ("a/b/converse/x", BedrockRoute::Converse),
            ("xconverse/x", BedrockRoute::Invoke),
            ("converse_like/x", BedrockRoute::ConverseLike),
            ("claude_platform/x", BedrockRoute::ClaudePlatform),
            ("converse/claude_platform/x", BedrockRoute::ClaudePlatform),
            ("xclaude_platform/x", BedrockRoute::Invoke),
            ("mantle/x", BedrockRoute::Mantle),
            ("x/mantle/y", BedrockRoute::Mantle),
            ("bedrock_mantle/x", BedrockRoute::Invoke),
            ("bedrock/chat_completions/x", BedrockRoute::ChatCompletions),
            ("xchat_completions/x", BedrockRoute::Invoke),
        ],
        |model| route.route(model, None).unwrap(),
    );
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
    model_rewrite_cases();
    prefix_predicate_cases();
    workspace_keys();
}

fn model_info_passthrough() {
    let price_map = ParamMap::new();
    let converse = IndexSet::new();
    let route = selector(&price_map, &converse, false);
    let bare = ParamMap::new();
    let supplied = params(&[
        ("api_base", text("https://base.example")),
        ("api_key", text("sk-fixture")),
        ("api_base_empty", text("")),
        (
            "extra_headers",
            mapping(&[("X-Amz-Target", text("BedrockRuntime.Converse"))]),
        ),
    ]);
    let original = supplied.clone();
    for model in [
        "converse/x",
        "invoke/x",
        "anthropic.claude-3",
        "unknown-model",
    ] {
        assert_eq!(
            route.route_for_request(model, &supplied, None).unwrap(),
            route.route_for_request(model, &bare, None).unwrap(),
            "{model}"
        );
    }
    assert_eq!(supplied, original);
}

fn reverse_tool_name_lookup() {
    let mappings = IndexMap::from([
        ("converted".to_owned(), "Original Name".to_owned()),
        ("x".to_owned(), "y".to_owned()),
        ("same".to_owned(), "same".to_owned()),
    ]);
    check_cases(
        &[
            ("converted", "Original Name".to_owned()),
            ("missing", "missing".to_owned()),
            ("", String::new()),
            ("same", "same".to_owned()),
            ("Original Name", "Original Name".to_owned()),
        ],
        |name| mappings.get(name).map_or(name, String::as_str).to_owned(),
    );
}
