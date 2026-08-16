use super::*;

/// Build a default `McpContext` and dispatch. Lets existing tests stay focused
/// on input/output without re-stating the no-op default options every time.
fn mcp_dispatch(method: &str, params: Option<Value>, path: &std::path::Path) -> JsonRpcResponse {
    let ctx = McpContext::new(
        path.to_path_buf(),
        McpOptions::default(),
        std::sync::Arc::new(crate::runtime::env::Env::empty()),
    );
    dispatch(method, params, &ctx)
}

/// Build a context with a custom `McpOptions` (read-only and/or audit log).
fn mcp_dispatch_with(
    method: &str,
    params: Option<Value>,
    path: &std::path::Path,
    options: McpOptions,
) -> (JsonRpcResponse, McpContext) {
    let ctx = McpContext::new(
        path.to_path_buf(),
        options,
        std::sync::Arc::new(crate::runtime::env::Env::empty()),
    );
    let resp = dispatch(method, params, &ctx);
    (resp, ctx)
}

// --- Task 1: JSON-RPC types and parsing ---

#[test]
fn parse_valid_request() {
    let json = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
    let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
    assert_eq!(req.method, "initialize");
    assert_eq!(req.id, Some(Value::Number(1.into())));
}

#[test]
fn parse_notification_no_id() {
    let json = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
    assert!(req.id.is_none());
    assert!(req.params.is_none());
}

#[test]
fn parse_invalid_json() {
    let result: Result<JsonRpcRequest, _> = serde_json::from_str("not json");
    assert!(result.is_err());
}

#[test]
fn response_success_serialization() {
    let resp = JsonRpcResponse::success(Some(Value::Number(1.into())), Value::Bool(true));
    let json = serde_json::to_string(&resp).unwrap();
    assert!(json.contains(r#""result":true"#));
    assert!(!json.contains("error"));
}

#[test]
fn response_error_serialization() {
    let resp = JsonRpcResponse::error(
        Some(Value::Number(1.into())),
        -32601,
        "Method not found".to_string(),
    );
    let json = serde_json::to_string(&resp).unwrap();
    assert!(json.contains("-32601"));
    assert!(!json.contains("result"));
}

// --- Task 2: MCP initialize and tools/list handlers ---

#[test]
fn test_handle_initialize() {
    let params = serde_json::json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {},
        "clientInfo": {"name": "test", "version": "1.0"}
    });
    let resp = mcp_dispatch(
        "initialize",
        Some(params),
        &std::path::PathBuf::from("/dev/null"),
    );
    let result = resp.result.unwrap();
    assert_eq!(result["protocolVersion"], "2024-11-05");
    assert!(result["capabilities"]["tools"].is_object());
    assert_eq!(result["serverInfo"]["name"], "purple");
}

#[test]
fn test_handle_tools_list() {
    let resp = mcp_dispatch("tools/list", None, &std::path::PathBuf::from("/dev/null"));
    let result = resp.result.unwrap();
    let tools = result["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 5);
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"list_hosts"));
    assert!(names.contains(&"get_host"));
    assert!(names.contains(&"run_command"));
    assert!(names.contains(&"list_containers"));
    assert!(names.contains(&"container_action"));
}

#[test]
fn test_handle_unknown_method() {
    let resp = mcp_dispatch("bogus/method", None, &std::path::PathBuf::from("/dev/null"));
    assert!(resp.error.is_some());
    assert_eq!(resp.error.unwrap().code, -32601);
}

// --- Task 3: list_hosts and get_host tool handlers ---

#[test]
fn tool_list_hosts_returns_all_concrete_hosts() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "list_hosts", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    let hosts: Vec<Value> = serde_json::from_str(text).unwrap();
    assert_eq!(hosts.len(), 2);
    assert_eq!(hosts[0]["alias"], "web-1");
    assert_eq!(hosts[1]["alias"], "db-1");
}

#[test]
fn tool_list_hosts_filter_by_tag() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({"tag": "database"});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "list_hosts", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    let hosts: Vec<Value> = serde_json::from_str(text).unwrap();
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0]["alias"], "db-1");
}

#[test]
fn tool_get_host_found() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({"alias": "web-1"});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "get_host", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    let host: Value = serde_json::from_str(text).unwrap();
    assert_eq!(host["alias"], "web-1");
    assert_eq!(host["hostname"], "10.0.1.5");
    assert_eq!(host["user"], "deploy");
    assert_eq!(host["identity_file"], "~/.ssh/id_ed25519");
    assert_eq!(host["provider"], "aws");
}

#[test]
fn tool_get_host_not_found() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({"alias": "nonexistent"});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "get_host", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
}

#[test]
fn tool_get_host_missing_alias() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "get_host", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
}

// --- Task 4: run_command tool handler ---

#[test]
fn tool_run_command_missing_alias() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({"command": "uptime"});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "run_command", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
}

#[test]
fn tool_run_command_missing_command() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({"alias": "web-1"});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "run_command", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
}

#[test]
fn tool_run_command_empty_alias() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({"alias": "", "command": "uptime"});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "run_command", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
}

#[test]
fn tool_run_command_empty_command() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({"alias": "web-1", "command": ""});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "run_command", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
}

// --- Task 5: list_containers and container_action tool handlers ---

#[test]
fn tool_list_containers_missing_alias() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "list_containers", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
}

#[test]
fn tool_container_action_missing_fields() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({"alias": "web-1", "action": "start"});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "container_action", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
}

#[test]
fn tool_container_action_invalid_action() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({"alias": "web-1", "container_id": "abc", "action": "destroy"});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "container_action", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
}

#[test]
fn tool_container_action_invalid_container_id() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args =
        serde_json::json!({"alias": "web-1", "container_id": "abc;rm -rf /", "action": "start"});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "container_action", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
}

// --- Protocol-level tests ---

#[test]
fn tools_call_missing_params() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let resp = mcp_dispatch("tools/call", None, &config_path);
    assert!(resp.result.is_none());
    let err = resp.error.unwrap();
    assert_eq!(err.code, -32602);
    assert!(err.message.contains("missing params"));
}

#[test]
fn tools_call_missing_tool_name() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"arguments": {}})),
        &config_path,
    );
    assert!(resp.result.is_none());
    let err = resp.error.unwrap();
    assert_eq!(err.code, -32602);
    assert!(err.message.contains("missing tool name"));
}

#[test]
fn tools_call_unknown_tool() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "nonexistent_tool", "arguments": {}})),
        &config_path,
    );
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Unknown tool")
    );
}

#[test]
fn tools_call_name_is_number_not_string() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": 42, "arguments": {}})),
        &config_path,
    );
    assert!(resp.result.is_none());
    let err = resp.error.unwrap();
    assert_eq!(err.code, -32602);
}

#[test]
fn tools_call_no_arguments_field() {
    // arguments defaults to {} when missing
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "list_hosts"})),
        &config_path,
    );
    let result = resp.result.unwrap();
    // Should succeed - list_hosts with no args returns all hosts
    assert!(result.get("isError").is_none());
    let text = result["content"][0]["text"].as_str().unwrap();
    let hosts: Vec<Value> = serde_json::from_str(text).unwrap();
    assert_eq!(hosts.len(), 2);
}

// --- list_hosts additional tests ---

#[test]
fn tool_list_hosts_empty_config() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_empty_config");
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "list_hosts", "arguments": {}})),
        &config_path,
    );
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    let hosts: Vec<Value> = serde_json::from_str(text).unwrap();
    assert!(hosts.is_empty());
}

#[test]
fn tool_list_hosts_filter_by_provider_name() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({"tag": "aws"});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "list_hosts", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    let hosts: Vec<Value> = serde_json::from_str(text).unwrap();
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0]["alias"], "web-1");
}

#[test]
fn tool_list_hosts_filter_case_insensitive() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({"tag": "PROD"});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "list_hosts", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    let hosts: Vec<Value> = serde_json::from_str(text).unwrap();
    assert_eq!(hosts.len(), 2); // both web-1 and db-1 have "prod" tag
}

#[test]
fn tool_list_hosts_filter_no_match() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({"tag": "nonexistent-tag"});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "list_hosts", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    let hosts: Vec<Value> = serde_json::from_str(text).unwrap();
    assert!(hosts.is_empty());
}

// Regression for the .mcpb-with-unexpanded-${HOME} bug. Before this guard
// existed the parser silently produced an empty config for missing files,
// and list_hosts returned `[]` with `isError: false`. The MCP client then
// presented "no hosts configured" to the user instead of surfacing the real
// problem.
#[test]
fn tool_list_hosts_missing_config_returns_explicit_error() {
    let config_path = std::path::PathBuf::from("/nonexistent/purple/mcp/test/path/ssh_config");
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "list_hosts", "arguments": {}})),
        &config_path,
    );
    let result = resp.result.unwrap();
    assert_eq!(
        result["isError"].as_bool(),
        Some(true),
        "missing config must surface as MCP-level error, not empty result"
    );
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("not found"),
        "error message must indicate the file is missing, got: {text}"
    );
}

#[test]
fn tool_get_host_missing_config_returns_explicit_error() {
    let config_path = std::path::PathBuf::from("/nonexistent/purple/mcp/test/path/ssh_config");
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({
            "name": "get_host",
            "arguments": {"alias": "anything"}
        })),
        &config_path,
    );
    let result = resp.result.unwrap();
    assert_eq!(result["isError"].as_bool(), Some(true));
}

#[test]
fn tool_list_hosts_filter_by_provider_tags() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_provider_tags_config");
    let args = serde_json::json!({"tag": "backend"});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "list_hosts", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    let hosts: Vec<Value> = serde_json::from_str(text).unwrap();
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0]["alias"], "tagged-1");
}

#[test]
fn tool_list_hosts_stale_field_is_boolean() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_stale_config");
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "list_hosts", "arguments": {}})),
        &config_path,
    );
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    let hosts: Vec<Value> = serde_json::from_str(text).unwrap();
    let stale_host = hosts.iter().find(|h| h["alias"] == "stale-1").unwrap();
    let active_host = hosts.iter().find(|h| h["alias"] == "active-1").unwrap();
    assert_eq!(stale_host["stale"], true);
    assert_eq!(active_host["stale"], false);
}

#[test]
fn tool_list_hosts_output_fields() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "list_hosts", "arguments": {}})),
        &config_path,
    );
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    let hosts: Vec<Value> = serde_json::from_str(text).unwrap();
    let host = &hosts[0];
    // Verify all expected fields are present
    assert!(host.get("alias").is_some());
    assert!(host.get("hostname").is_some());
    assert!(host.get("user").is_some());
    assert!(host.get("port").is_some());
    assert!(host.get("tags").is_some());
    assert!(host.get("provider").is_some());
    assert!(host.get("stale").is_some());
    // Verify types
    assert!(host["port"].is_number());
    assert!(host["tags"].is_array());
    assert!(host["stale"].is_boolean());
}

// --- get_host additional tests ---

#[test]
fn tool_get_host_rejects_empty_alias() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({"alias": ""});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "get_host", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    assert_eq!(result["isError"], true);
    let text = result["content"][0]["text"].as_str().unwrap();
    assert_eq!(text, "Missing required parameter: alias");
}

#[test]
fn tool_get_host_alias_is_number() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({"alias": 42});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "get_host", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
}

#[test]
fn tool_get_host_output_fields() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({"alias": "web-1"});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "get_host", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    let host: Value = serde_json::from_str(text).unwrap();
    // Verify all expected fields
    assert_eq!(host["port"], 22);
    assert!(host["tags"].is_array());
    assert!(host["provider_tags"].is_array());
    assert!(host["provider_meta"].is_object());
    assert!(host["stale"].is_boolean());
    assert_eq!(host["stale"], false);
    assert_eq!(host["tunnel_count"], 0);
    // Verify provider_meta content
    assert_eq!(host["provider_meta"]["region"], "us-east-1");
    assert_eq!(host["provider_meta"]["instance"], "t3.micro");
}

#[test]
fn tool_get_host_no_provider() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({"alias": "db-1"});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "get_host", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    let host: Value = serde_json::from_str(text).unwrap();
    assert!(host["provider"].is_null());
    assert!(host["provider_meta"].as_object().unwrap().is_empty());
    assert_eq!(host["port"], 5432);
}

#[test]
fn tool_get_host_stale_is_boolean() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_stale_config");
    let args = serde_json::json!({"alias": "stale-1"});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "get_host", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    let host: Value = serde_json::from_str(text).unwrap();
    assert_eq!(host["stale"], true);
}

#[test]
fn tool_get_host_case_sensitive() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({"alias": "WEB-1"});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "get_host", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
}

// --- run_command additional tests ---

#[test]
fn tool_run_command_nonexistent_alias() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({"alias": "nonexistent-host", "command": "uptime"});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "run_command", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("not found")
    );
}

#[test]
fn tool_run_command_alias_is_number() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({"alias": 42, "command": "uptime"});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "run_command", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
}

#[test]
fn tool_run_command_rejects_non_string_command() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({"alias": "web-1", "command": 123});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "run_command", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
}

#[test]
fn tool_run_command_timeout_string_falls_back_to_default() {
    // Use a deliberately non-existent alias so the test never invokes ssh
    // even if the local fixture name resolves on the host machine.
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({
        "alias": "nonexistent-host",
        "command": "uptime",
        "timeout": "not-a-number"
    });
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "run_command", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    assert_eq!(result["isError"], true);
    // The error should be the alias-not-found path, not an input validation
    // error - which proves the bad timeout was tolerated.
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("not found"), "got: {text}");
}

// --- container_action additional tests ---

#[test]
fn tool_container_action_empty_alias() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({"alias": "", "container_id": "abc", "action": "start"});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "container_action", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
}

#[test]
fn tool_container_action_empty_container_id() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({"alias": "web-1", "container_id": "", "action": "start"});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "container_action", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
}

#[test]
fn tool_container_action_nonexistent_alias() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args =
        serde_json::json!({"alias": "nonexistent", "container_id": "abc", "action": "start"});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "container_action", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("not found")
    );
}

#[test]
fn tool_container_action_uppercase_action() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({"alias": "web-1", "container_id": "abc", "action": "START"});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "container_action", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Invalid action")
    );
}

#[test]
fn tool_container_action_container_id_with_dots_and_hyphens() {
    // Valid container IDs can have dots, hyphens, underscores
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({"alias": "web-1", "container_id": "my-container_v1.2", "action": "start"});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "container_action", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    // Should NOT error on validation - container_id is valid
    // Will proceed to alias check and SSH (which may fail), but no validation error
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(!text.contains("invalid character"));
}

#[test]
fn tool_container_action_container_id_with_spaces() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args =
        serde_json::json!({"alias": "web-1", "container_id": "my container", "action": "start"});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "container_action", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("invalid character")
    );
}

#[test]
fn tool_list_containers_rejects_empty_alias() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({"alias": ""});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "list_containers", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
}

#[test]
fn tool_list_containers_nonexistent_alias() {
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({"alias": "nonexistent"});
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "list_containers", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    assert!(result["isError"].as_bool().unwrap());
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("not found")
    );
}

// --- initialize and tools/list output tests ---

#[test]
fn initialize_contains_version() {
    let resp = mcp_dispatch("initialize", None, &std::path::PathBuf::from("/dev/null"));
    let result = resp.result.unwrap();
    assert!(!result["serverInfo"]["version"].as_str().unwrap().is_empty());
}

#[test]
fn tools_list_schema_has_required_fields() {
    let resp = mcp_dispatch("tools/list", None, &std::path::PathBuf::from("/dev/null"));
    let result = resp.result.unwrap();
    let tools = result["tools"].as_array().unwrap();
    for tool in tools {
        assert!(tool["name"].is_string(), "Tool missing name");
        assert!(tool["description"].is_string(), "Tool missing description");
        assert!(tool["inputSchema"].is_object(), "Tool missing inputSchema");
        assert_eq!(tool["inputSchema"]["type"], "object");
    }
}

#[test]
fn every_tool_has_annotations_required_by_directory_submission() {
    // Anthropic's Desktop Extension submission requires every tool to expose
    // a `title` and the appropriate `readOnlyHint` or `destructiveHint`. The
    // hints must agree with the READ_ONLY_TOOLS allowlist: a tool is in the
    // allowlist iff `readOnlyHint == true` and `destructiveHint == false`.
    let resp = mcp_dispatch("tools/list", None, &std::path::PathBuf::from("/dev/null"));
    let result = resp.result.unwrap();
    let tools = result["tools"].as_array().unwrap();
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        let ann = &tool["annotations"];
        assert!(
            ann.is_object(),
            "{name} is missing the annotations object required for directory submission"
        );
        let title = ann["title"].as_str();
        assert!(
            title.is_some_and(|s| s.len() >= 5 && !s.contains('_')),
            "{name} annotations.title must be a human readable string (>=5 chars, no underscores), got {title:?}"
        );
        let read_only = ann["readOnlyHint"]
            .as_bool()
            .expect("readOnlyHint must be a bool");
        let destructive = ann["destructiveHint"]
            .as_bool()
            .expect("destructiveHint must be a bool");
        let in_allowlist = READ_ONLY_TOOLS.contains(&name);
        assert_eq!(
            in_allowlist,
            read_only && !destructive,
            "{name} hints disagree with READ_ONLY_TOOLS allowlist (read_only={read_only}, destructive={destructive}, in_allowlist={in_allowlist})"
        );
    }
}

#[test]
fn tool_annotations_have_exact_per_tool_values() {
    // Double-entry ledger: a bug that flips both READ_ONLY_TOOLS and the
    // annotations together would slip past the consistency check. This test
    // pins each tool's hints to the exact values we publish to the directory.
    // (tool_name, read_only, destructive, idempotent)
    let expected = [
        ("list_hosts", true, false, true),
        ("get_host", true, false, true),
        ("list_containers", true, false, true),
        ("run_command", false, true, false),
        ("container_action", false, true, false),
    ];
    let resp = mcp_dispatch("tools/list", None, &std::path::PathBuf::from("/dev/null"));
    let tools = resp.result.unwrap()["tools"].as_array().cloned().unwrap();
    assert_eq!(tools.len(), expected.len());
    for (name, ro, destr, idem) in expected {
        let tool = tools
            .iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("missing tool {name}"));
        let ann = &tool["annotations"];
        assert_eq!(ann["readOnlyHint"], ro, "{name}.readOnlyHint");
        assert_eq!(ann["destructiveHint"], destr, "{name}.destructiveHint");
        assert_eq!(ann["idempotentHint"], idem, "{name}.idempotentHint");
    }
}

// --- Read-only mode tests ---

#[test]
fn read_only_filters_state_changing_tools_from_list() {
    let opts = McpOptions {
        read_only: true,
        audit_log_path: None,
    };
    let (resp, _ctx) = mcp_dispatch_with(
        "tools/list",
        None,
        &std::path::PathBuf::from("/dev/null"),
        opts,
    );
    let result = resp.result.unwrap();
    let tools = result["tools"].as_array().unwrap();
    let mut names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    names.sort();
    let mut expected: Vec<&str> = READ_ONLY_TOOLS.to_vec();
    expected.sort();
    assert_eq!(
        names, expected,
        "read-only tools/list must match the READ_ONLY_TOOLS allowlist"
    );
}

#[test]
fn read_only_denies_run_command() {
    let opts = McpOptions {
        read_only: true,
        audit_log_path: None,
    };
    let (resp, _ctx) = mcp_dispatch_with(
        "tools/call",
        Some(serde_json::json!({
            "name": "run_command",
            "arguments": {"alias": "web-1", "command": "uptime"}
        })),
        &std::path::PathBuf::from("tests/fixtures/mcp_test_config"),
        opts,
    );
    let result = resp.result.unwrap();
    assert_eq!(result["isError"], true);
    let text = result["content"][0]["text"].as_str().unwrap();
    assert_eq!(text, crate::messages::MCP_TOOL_DENIED_READ_ONLY);
}

#[test]
fn read_only_denies_container_action() {
    let opts = McpOptions {
        read_only: true,
        audit_log_path: None,
    };
    let (resp, _ctx) = mcp_dispatch_with(
        "tools/call",
        Some(serde_json::json!({
            "name": "container_action",
            "arguments": {"alias": "web-1", "container_id": "abc", "action": "start"}
        })),
        &std::path::PathBuf::from("tests/fixtures/mcp_test_config"),
        opts,
    );
    let result = resp.result.unwrap();
    assert_eq!(result["isError"], true);
    let text = result["content"][0]["text"].as_str().unwrap();
    assert_eq!(text, crate::messages::MCP_TOOL_DENIED_READ_ONLY);
}

#[test]
fn read_only_gates_before_argument_validation() {
    // The read-only guard must fire BEFORE the per-tool argument validation,
    // otherwise an attacker probing in read-only mode could distinguish
    // "tool denied" from "tool would have been called but args were bad",
    // leaking information about which tools the server supports.
    let opts = McpOptions {
        read_only: true,
        audit_log_path: None,
    };
    let (resp, _ctx) = mcp_dispatch_with(
        "tools/call",
        Some(serde_json::json!({
            "name": "container_action",
            "arguments": {"alias": "web-1", "container_id": "abc", "action": "nuke"}
        })),
        &std::path::PathBuf::from("tests/fixtures/mcp_test_config"),
        opts,
    );
    let result = resp.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert_eq!(
        text,
        crate::messages::MCP_TOOL_DENIED_READ_ONLY,
        "read-only must trump argument validation"
    );
    assert!(!text.contains("Invalid action"));
}

#[test]
fn read_only_allows_list_hosts() {
    let opts = McpOptions {
        read_only: true,
        audit_log_path: None,
    };
    let (resp, _ctx) = mcp_dispatch_with(
        "tools/call",
        Some(serde_json::json!({"name": "list_hosts", "arguments": {}})),
        &std::path::PathBuf::from("tests/fixtures/mcp_test_config"),
        opts,
    );
    let result = resp.result.unwrap();
    assert!(
        result.get("isError").is_none(),
        "list_hosts should succeed in read-only mode, got: {result}"
    );
}

#[test]
fn read_only_allows_get_host() {
    let opts = McpOptions {
        read_only: true,
        audit_log_path: None,
    };
    let (resp, _ctx) = mcp_dispatch_with(
        "tools/call",
        Some(serde_json::json!({"name": "get_host", "arguments": {"alias": "web-1"}})),
        &std::path::PathBuf::from("tests/fixtures/mcp_test_config"),
        opts,
    );
    let result = resp.result.unwrap();
    assert!(result.get("isError").is_none());
}

// --- Audit log tests ---

/// Parse all JSON Lines in an audit log into structured `Value`s.
/// Failing to parse any line is an assertion failure: the audit log MUST
/// remain valid JSON Lines under all inputs.
fn read_audit_entries(path: &std::path::Path) -> Vec<Value> {
    let contents = std::fs::read_to_string(path).unwrap();
    contents
        .lines()
        .map(|line| {
            serde_json::from_str::<Value>(line)
                .unwrap_or_else(|e| panic!("malformed audit line {line:?}: {e}"))
        })
        .collect()
}

#[test]
fn audit_log_records_allowed_call() {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("audit.log");
    let opts = McpOptions {
        read_only: false,
        audit_log_path: Some(log_path.clone()),
    };
    let (_resp, _ctx) = mcp_dispatch_with(
        "tools/call",
        Some(serde_json::json!({"name": "list_hosts", "arguments": {}})),
        &std::path::PathBuf::from("tests/fixtures/mcp_test_config"),
        opts,
    );
    let entries = read_audit_entries(&log_path);
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(e["tool"], "list_hosts");
    assert_eq!(e["outcome"], "allowed");
    // The reason field MUST be present and null on allowed entries so SIEM
    // ingest can rely on a stable schema.
    assert!(e.get("reason").is_some(), "reason field must be present");
    assert!(e["reason"].is_null(), "reason must be null on allowed");
    assert!(e["ts"].is_string());
}

#[test]
fn audit_log_records_denied_call_in_read_only_mode() {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("audit.log");
    let opts = McpOptions {
        read_only: true,
        audit_log_path: Some(log_path.clone()),
    };
    let (_resp, _ctx) = mcp_dispatch_with(
        "tools/call",
        Some(serde_json::json!({
            "name": "run_command",
            "arguments": {"alias": "web-1", "command": "uptime"}
        })),
        &std::path::PathBuf::from("tests/fixtures/mcp_test_config"),
        opts,
    );
    let entries = read_audit_entries(&log_path);
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(e["tool"], "run_command");
    assert_eq!(e["outcome"], "denied");
    assert_eq!(e["reason"], "read-only mode");
}

#[test]
fn audit_log_records_error_outcome() {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("audit.log");
    let opts = McpOptions {
        read_only: false,
        audit_log_path: Some(log_path.clone()),
    };
    let (_resp, _ctx) = mcp_dispatch_with(
        "tools/call",
        Some(serde_json::json!({
            "name": "get_host",
            "arguments": {"alias": "does-not-exist"}
        })),
        &std::path::PathBuf::from("tests/fixtures/mcp_test_config"),
        opts,
    );
    let entries = read_audit_entries(&log_path);
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(e["outcome"], "error");
    assert!(e["reason"].is_null(), "error outcomes carry no reason");
}

#[test]
fn audit_log_appends_multiple_entries() {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("audit.log");
    let opts = McpOptions {
        read_only: false,
        audit_log_path: Some(log_path.clone()),
    };
    let ctx = McpContext::new(
        std::path::PathBuf::from("tests/fixtures/mcp_test_config"),
        opts,
        std::sync::Arc::new(crate::runtime::env::Env::empty()),
    );
    for _ in 0..3 {
        dispatch(
            "tools/call",
            Some(serde_json::json!({"name": "list_hosts", "arguments": {}})),
            &ctx,
        );
    }
    let entries = read_audit_entries(&log_path);
    assert_eq!(entries.len(), 3);
    for e in &entries {
        assert_eq!(e["tool"], "list_hosts");
        assert_eq!(e["outcome"], "allowed");
    }
}

#[test]
fn audit_log_handles_concurrent_writes() {
    // The AuditLog's `Mutex<File>` claim in the doc comment is only meaningful
    // if it actually holds under concurrent writers. Spawn N threads each
    // writing one entry, then verify N well-formed lines with no truncation
    // or interleaving.
    use std::sync::Arc;
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("audit.log");
    let ctx = Arc::new(McpContext::new(
        std::path::PathBuf::from("tests/fixtures/mcp_test_config"),
        McpOptions {
            read_only: false,
            audit_log_path: Some(log_path.clone()),
        },
        std::sync::Arc::new(crate::runtime::env::Env::empty()),
    ));
    let handles: Vec<_> = (0..16)
        .map(|i| {
            let ctx = Arc::clone(&ctx);
            std::thread::spawn(move || {
                dispatch(
                    "tools/call",
                    Some(serde_json::json!({
                        "name": "list_hosts",
                        "arguments": {"tag": format!("tag-{i}")}
                    })),
                    &ctx,
                );
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let entries = read_audit_entries(&log_path);
    assert_eq!(entries.len(), 16);
    for e in &entries {
        assert_eq!(e["tool"], "list_hosts");
        assert_eq!(e["outcome"], "allowed");
    }
}

#[test]
fn audit_log_disabled_when_no_path() {
    let opts = McpOptions {
        read_only: false,
        audit_log_path: None,
    };
    let (_resp, ctx) = mcp_dispatch_with(
        "tools/call",
        Some(serde_json::json!({"name": "list_hosts", "arguments": {}})),
        &std::path::PathBuf::from("tests/fixtures/mcp_test_config"),
        opts,
    );
    assert!(ctx.audit.is_none());
}

#[test]
fn audit_log_creates_parent_directory() {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("nested").join("subdir").join("audit.log");
    let opts = McpOptions {
        read_only: false,
        audit_log_path: Some(log_path.clone()),
    };
    let (_resp, _ctx) = mcp_dispatch_with(
        "tools/call",
        Some(serde_json::json!({"name": "list_hosts", "arguments": {}})),
        &std::path::PathBuf::from("tests/fixtures/mcp_test_config"),
        opts,
    );
    assert!(log_path.exists());
}

#[cfg(unix)]
#[test]
fn audit_log_init_failure_does_not_break_dispatch() {
    use std::os::unix::fs::PermissionsExt;
    // Skip under root (e.g. inside CI Docker containers): root ignores
    // filesystem mode bits, so the 0o555 parent below would not actually
    // block file creation and the assertion would fire spuriously.
    // SAFETY: getuid() is a thread-safe POSIX call with no preconditions.
    if unsafe { libc::getuid() } == 0 {
        return;
    }
    // Build an unwriteable parent in a tempdir so the test is portable
    // across Linux and macOS (no /proc dependency).
    let dir = tempfile::tempdir().unwrap();
    let ro_parent = dir.path().join("ro");
    std::fs::create_dir(&ro_parent).unwrap();
    std::fs::set_permissions(&ro_parent, std::fs::Permissions::from_mode(0o555)).unwrap();
    let opts = McpOptions {
        read_only: false,
        audit_log_path: Some(ro_parent.join("audit.log")),
    };
    let (resp, ctx) = mcp_dispatch_with(
        "tools/call",
        Some(serde_json::json!({"name": "list_hosts", "arguments": {}})),
        &std::path::PathBuf::from("tests/fixtures/mcp_test_config"),
        opts,
    );
    // The dispatch should still succeed even though audit log failed to open.
    assert!(resp.result.is_some());
    assert!(
        ctx.audit.is_none(),
        "audit should be None after init failure"
    );
    // Restore permissions so tempdir cleanup works.
    let _ = std::fs::set_permissions(&ro_parent, std::fs::Permissions::from_mode(0o755));
}

#[test]
fn audit_log_records_allowed_call_in_read_only_mode() {
    // Closes the gap where read-only + audit + an allowed tool was not
    // exercised together. Earlier tests covered each pair but not the trio.
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("audit.log");
    let opts = McpOptions {
        read_only: true,
        audit_log_path: Some(log_path.clone()),
    };
    let (_resp, _ctx) = mcp_dispatch_with(
        "tools/call",
        Some(serde_json::json!({"name": "list_hosts", "arguments": {}})),
        &std::path::PathBuf::from("tests/fixtures/mcp_test_config"),
        opts,
    );
    let entries = read_audit_entries(&log_path);
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(e["tool"], "list_hosts");
    assert_eq!(e["outcome"], "allowed");
    assert!(e["reason"].is_null());
}

#[test]
fn audit_log_appends_to_existing_file() {
    // Pre-existing entries must survive: AuditLog::open uses append(true).
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("audit.log");
    std::fs::write(&log_path, "{\"pre\":\"existing\"}\n").unwrap();
    let opts = McpOptions {
        read_only: false,
        audit_log_path: Some(log_path.clone()),
    };
    let (_resp, _ctx) = mcp_dispatch_with(
        "tools/call",
        Some(serde_json::json!({"name": "list_hosts", "arguments": {}})),
        &std::path::PathBuf::from("tests/fixtures/mcp_test_config"),
        opts,
    );
    let entries = read_audit_entries(&log_path);
    assert_eq!(entries.len(), 2, "pre-existing line + new entry");
    assert_eq!(entries[0]["pre"], "existing");
    assert_eq!(entries[1]["tool"], "list_hosts");
}

#[test]
fn audit_log_redacts_run_command_command() {
    // The `command` field for run_command may carry secrets. The audit log
    // records that the tool was called and on which host, not the literal
    // command body.
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("audit.log");
    let opts = McpOptions {
        read_only: false,
        audit_log_path: Some(log_path.clone()),
    };
    let (_resp, _ctx) = mcp_dispatch_with(
        "tools/call",
        Some(serde_json::json!({
            "name": "run_command",
            "arguments": {"alias": "nonexistent", "command": "mysql -pSUPERSECRET"}
        })),
        &std::path::PathBuf::from("tests/fixtures/mcp_test_config"),
        opts,
    );
    let entries = read_audit_entries(&log_path);
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(e["args"]["command"], "<redacted>");
    assert_eq!(e["args"]["alias"], "nonexistent");
    let raw = std::fs::read_to_string(&log_path).unwrap();
    assert!(!raw.contains("SUPERSECRET"), "secret leaked: {raw}");
}

#[test]
fn audit_log_redacts_run_command_when_args_is_not_an_object() {
    // Defensive: if a malformed client sends `arguments` as an array or
    // string for run_command, the secret could land anywhere inside that
    // value. Redact the whole payload.
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("audit.log");
    let opts = McpOptions {
        read_only: false,
        audit_log_path: Some(log_path.clone()),
    };
    let (_resp, _ctx) = mcp_dispatch_with(
        "tools/call",
        Some(serde_json::json!({
            "name": "run_command",
            "arguments": ["mysql -pSUPERSECRET", "--force"]
        })),
        &std::path::PathBuf::from("tests/fixtures/mcp_test_config"),
        opts,
    );
    let raw = std::fs::read_to_string(&log_path).unwrap();
    assert!(
        !raw.contains("SUPERSECRET"),
        "secret leaked via non-object args: {raw}"
    );
}

#[test]
fn audit_log_does_not_redact_other_tools() {
    // Only run_command's command field is sensitive. list_hosts args (which
    // include a tag filter) and others should remain visible.
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("audit.log");
    let opts = McpOptions {
        read_only: false,
        audit_log_path: Some(log_path.clone()),
    };
    let (_resp, _ctx) = mcp_dispatch_with(
        "tools/call",
        Some(serde_json::json!({"name": "list_hosts", "arguments": {"tag": "prod"}})),
        &std::path::PathBuf::from("tests/fixtures/mcp_test_config"),
        opts,
    );
    let entries = read_audit_entries(&log_path);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["args"]["tag"], "prod");
}

#[cfg(unix)]
#[test]
fn audit_log_file_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("audit.log");
    let opts = McpOptions {
        read_only: false,
        audit_log_path: Some(log_path.clone()),
    };
    let _ = mcp_dispatch_with(
        "tools/call",
        Some(serde_json::json!({"name": "list_hosts", "arguments": {}})),
        &std::path::PathBuf::from("tests/fixtures/mcp_test_config"),
        opts,
    );
    let mode = std::fs::metadata(&log_path).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o600,
        "audit log must be owner read/write only, got {mode:o}"
    );
}

// --- iso8601 helpers ---

#[test]
fn iso8601_format_known_timestamp() {
    // 2026-04-19T00:00:00Z = 1776556800 seconds since epoch
    assert_eq!(format_iso8601_utc(1_776_556_800), "2026-04-19T00:00:00Z");
}

#[test]
fn iso8601_format_epoch() {
    assert_eq!(format_iso8601_utc(0), "1970-01-01T00:00:00Z");
}

#[test]
fn iso8601_format_includes_seconds() {
    // 2026-04-19T12:34:56Z = 1776602096
    assert_eq!(format_iso8601_utc(1_776_602_096), "2026-04-19T12:34:56Z");
}

#[test]
fn iso8601_format_leap_day() {
    // 2024-02-29T00:00:00Z = 1709164800
    assert_eq!(format_iso8601_utc(1_709_164_800), "2024-02-29T00:00:00Z");
}

#[test]
fn iso8601_format_year_2000() {
    // 2000-01-01T00:00:00Z = 946684800
    assert_eq!(format_iso8601_utc(946_684_800), "2000-01-01T00:00:00Z");
}

#[test]
fn iso8601_format_non_leap_century() {
    // 2100 is divisible by 100 but not by 400, so NOT a leap year per Gregorian.
    // 2100-03-01T00:00:00Z = 4107542400.
    // If the algorithm wrongly treated 2100 as leap (Feb 29 inserted), this
    // would render "2100-02-29..." instead.
    assert_eq!(format_iso8601_utc(4_107_542_400), "2100-03-01T00:00:00Z");
}

// --- Additional safety tests ---

#[test]
fn audit_log_redacts_run_command_when_args_is_a_string() {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("audit.log");
    let opts = McpOptions {
        read_only: false,
        audit_log_path: Some(log_path.clone()),
    };
    let (_resp, _ctx) = mcp_dispatch_with(
        "tools/call",
        Some(serde_json::json!({
            "name": "run_command",
            "arguments": "mysql -pSUPERSECRET"
        })),
        &std::path::PathBuf::from("tests/fixtures/mcp_test_config"),
        opts,
    );
    let raw = std::fs::read_to_string(&log_path).unwrap();
    assert!(
        !raw.contains("SUPERSECRET"),
        "secret leaked via string args: {raw}"
    );
}

#[cfg(unix)]
#[test]
fn audit_log_refuses_symlink_target() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real.log");
    std::fs::write(&real, b"existing\n").unwrap();
    let link = dir.path().join("link.log");
    symlink(&real, &link).unwrap();

    let opts = McpOptions {
        read_only: false,
        audit_log_path: Some(link.clone()),
    };
    let (resp, ctx) = mcp_dispatch_with(
        "tools/call",
        Some(serde_json::json!({"name": "list_hosts", "arguments": {}})),
        &std::path::PathBuf::from("tests/fixtures/mcp_test_config"),
        opts,
    );
    assert!(
        resp.result.is_some(),
        "dispatch should not break on symlink refusal"
    );
    assert!(
        ctx.audit.is_none(),
        "symlink target must produce a None audit handle"
    );
    // The original file content must be untouched (we never opened it).
    let real_after = std::fs::read_to_string(&real).unwrap();
    assert_eq!(real_after, "existing\n");
}

#[test]
fn run_command_timeout_clamps_to_max_300() {
    // We cannot easily observe the internal clamped value via dispatch, so we
    // exercise the path with an unreachable host. The clamp guarantees that a
    // huge `timeout` does not actually affect this code path either way -
    // assertion is that the call returns an error WITHOUT hanging the test.
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({
        "alias": "nonexistent-host",
        "command": "uptime",
        "timeout": 99_999_999u64
    });
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "run_command", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    assert_eq!(result["isError"], true);
}

#[test]
fn run_command_timeout_clamps_zero_to_one() {
    // A timeout of 0 would cause the wait to fire immediately. The clamp
    // floors to 1 second, giving the not-found path room to return.
    let config_path = std::path::PathBuf::from("tests/fixtures/mcp_test_config");
    let args = serde_json::json!({
        "alias": "nonexistent-host",
        "command": "uptime",
        "timeout": 0u64
    });
    let resp = mcp_dispatch(
        "tools/call",
        Some(serde_json::json!({"name": "run_command", "arguments": args})),
        &config_path,
    );
    let result = resp.result.unwrap();
    assert_eq!(result["isError"], true);
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("not found"), "got: {text}");
}

// ── default_audit_log_path branches ──────────────────────────────────

#[test]
fn default_audit_log_path_lives_in_the_state_directory() {
    let legacy = crate::runtime::env::Paths::new("/var/test/home/eric");
    assert_eq!(
        super::default_audit_log_path(Some(&legacy)),
        Some(std::path::PathBuf::from(
            "/var/test/home/eric/.purple/mcp-audit.log"
        ))
    );
    let split = crate::runtime::env::Paths::resolve("/var/test/home/eric", |k| {
        (k == "XDG_STATE_HOME").then(|| "/var/test/state".to_string())
    });
    assert_eq!(
        super::default_audit_log_path(Some(&split)),
        Some(std::path::PathBuf::from(
            "/var/test/state/purple/mcp-audit.log"
        ))
    );
}

#[test]
fn default_audit_log_path_none_without_home_returns_none_silently() {
    // The warn! is fire-and-forget; we just verify the return value.
    // In production this disables auditing without crashing the server.
    assert_eq!(super::default_audit_log_path(None), None);
}
