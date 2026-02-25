use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{self, BufRead, BufReader, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::RwLock;

// ==================== MCP Protocol Types ====================

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    #[serde(default)]
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

// ==================== OpenAPI Data Structures ====================

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenApiSpec {
    pub openapi: String,
    pub info: Info,
    #[serde(default)]
    pub servers: Vec<Server>,
    #[serde(default)]
    pub paths: HashMap<String, PathItem>,
    #[serde(default)]
    pub components: Option<Components>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    pub title: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    pub url: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PathItem {
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub get: Option<Operation>,
    #[serde(default)]
    pub post: Option<Operation>,
    #[serde(default)]
    pub put: Option<Operation>,
    #[serde(default)]
    pub delete: Option<Operation>,
    #[serde(default)]
    pub patch: Option<Operation>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Operation {
    #[serde(default)]
    pub operation_id: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub parameters: Option<Vec<Parameter>>,
    #[serde(default)]
    pub request_body: Option<RequestBody>,
    #[serde(default)]
    pub responses: Option<HashMap<String, Response>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Parameter {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub r#in: String,
    #[serde(default)]
    pub required: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestBody {
    #[serde(default)]
    pub description: Option<String>,
    pub content: HashMap<String, MediaType>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MediaType {
    #[serde(default)]
    pub schema: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Response {
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Components {
    #[serde(default)]
    pub schemas: Option<HashMap<String, Value>>,
}

// ==================== MCP Server Handler ====================

struct OpenApiHandler {
    spec: Arc<RwLock<Option<OpenApiSpec>>>,
}

impl OpenApiHandler {
    fn new() -> Self {
        Self {
            spec: Arc::new(RwLock::new(None)),
        }
    }

    async fn load_spec(&self, path: &Path) -> Result<String> {
        let yaml_content = tokio::fs::read_to_string(path).await?;
        let spec: OpenApiSpec = serde_yaml::from_str(&yaml_content)?;
        {
            let mut spec_guard = self.spec.write().await;
            *spec_guard = Some(spec);
        }
        Ok(format!("OpenAPI spec loaded successfully"))
    }

    async fn list_apis(&self) -> Result<String> {
        let spec = self.spec.read().await;
        let spec = spec.as_ref().ok_or_else(|| anyhow::anyhow!("OpenAPI spec is not loaded"))?;

        let mut result = String::new();
        result.push_str(&format!("# {}\n\n", spec.info.title));

        for (path, path_item) in &spec.paths {
            result.push_str(&format!("## {}\n", path));
            for (method, op) in [
                ("GET", &path_item.get),
                ("POST", &path_item.post),
                ("PUT", &path_item.put),
                ("DELETE", &path_item.delete),
                ("PATCH", &path_item.patch),
            ] {
                if let Some(operation) = op {
                    result.push_str(&format!("  - {}", method));
                    if let Some(summary) = &operation.summary {
                        result.push_str(&format!(": {}", summary));
                    }
                    result.push('\n');
                }
            }
            result.push('\n');
        }
        Ok(result)
    }

    async fn get_api(&self, path: &str, method: &str) -> Result<String> {
        let spec = self.spec.read().await;
        let spec = spec.as_ref().ok_or_else(|| anyhow::anyhow!("OpenAPI spec is not loaded"))?;

        let path_item = spec.paths.get(path)
            .ok_or_else(|| anyhow::anyhow!("Path not found: {}", path))?;

        let operation = match method.to_uppercase().as_str() {
            "GET" => path_item.get.as_ref(),
            "POST" => path_item.post.as_ref(),
            "PUT" => path_item.put.as_ref(),
            "DELETE" => path_item.delete.as_ref(),
            "PATCH" => path_item.patch.as_ref(),
            _ => return Err(anyhow::anyhow!("Unsupported HTTP method: {}", method)),
        };

        let operation = operation.ok_or_else(|| anyhow::anyhow!("Path {} does not have method {}", path, method))?;

        let mut result = String::new();
        result.push_str(&format!("# {} {}\n\n", method.to_uppercase(), path));

        if let Some(summary) = &operation.summary {
            result.push_str(&format!("**Summary:** {}\n\n", summary));
        }

        if let Some(params) = &operation.parameters {
            result.push_str("**Parameters:**\n");
            for param in params {
                result.push_str(&format!("- `{}` ({})", param.name, param.r#in));
                if param.required.unwrap_or(false) {
                    result.push_str(" **[Required]**");
                }
                result.push('\n');
            }
            result.push('\n');
        }

        Ok(result)
    }

    async fn search_apis(&self, keyword: &str) -> Result<String> {
        let spec = self.spec.read().await;
        let spec = spec.as_ref().ok_or_else(|| anyhow::anyhow!("OpenAPI spec is not loaded"))?;

        let keyword_lower = keyword.to_lowercase();
        let mut results = Vec::new();

        for (path, path_item) in &spec.paths {
            let mut matched = false;
            let mut methods = Vec::new();

            for (method_name, operation) in [
                ("GET", &path_item.get),
                ("POST", &path_item.post),
                ("PUT", &path_item.put),
                ("DELETE", &path_item.delete),
                ("PATCH", &path_item.patch),
            ] {
                if let Some(op) = operation {
                    let summary = op.summary.as_ref().map(|s| s.to_lowercase()).unwrap_or_default();
                    if path.to_lowercase().contains(&keyword_lower) || summary.contains(&keyword_lower) {
                        matched = true;
                        methods.push((method_name.to_string(), op.summary.clone().unwrap_or_default()));
                    }
                }
            }

            if matched {
                results.push((path.clone(), methods));
            }
        }

        if results.is_empty() {
            return Ok(format!("No APIs found containing \"{}\"", keyword));
        }

        let mut result = String::new();
        result.push_str(&format!("Search for \"{}\" returned {} result(s):\n\n", keyword, results.len()));
        for (path, methods) in results {
            result.push_str(&format!("## {}\n", path));
            for (method, summary) in methods {
                result.push_str(&format!("  - {}: {}\n", method, summary));
            }
            result.push('\n');
        }
        Ok(result)
    }

    async fn get_servers(&self) -> Result<String> {
        let spec = self.spec.read().await;
        let spec = spec.as_ref().ok_or_else(|| anyhow::anyhow!("OpenAPI spec is not loaded"))?;

        if spec.servers.is_empty() {
            return Ok("No servers defined".to_string());
        }

        let mut result = String::new();
        result.push_str("**Servers:**\n\n");
        for server in &spec.servers {
            result.push_str(&format!("- URL: {}\n", server.url));
            if let Some(desc) = &server.description {
                result.push_str(&format!("  Description: {}\n", desc));
            }
        }
        Ok(result)
    }
}

// ==================== Main ====================

#[tokio::main]
async fn main() -> Result<()> {
    eprintln!("OpenAPI MCP Server starting...");

    let handler = OpenApiHandler::new();

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut reader = BufReader::new(stdin);
    let mut line = String::new();

    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line)?;
        if bytes_read == 0 {
            break;
        }

        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        match serde_json::from_str::<JsonRpcRequest>(line) {
            Ok(req) => {
                let response = handle_request(&handler, req).await;
                let output = serde_json::to_string(&response)?;
                writeln!(stdout, "{}", output)?;
                stdout.flush()?;
            }
            Err(e) => {
                eprintln!("Failed to parse request: {}", e);
            }
        }
    }

    Ok(())
}

async fn handle_request(handler: &OpenApiHandler, req: JsonRpcRequest) -> JsonRpcResponse {
    let result = match req.method.as_str() {
        "initialize" => {
            json!({
                "protocolVersion": "2024-11-05",
                "serverInfo": {
                    "name": "openapi-mcp-server",
                    "version": "0.1.0"
                },
                "capabilities": {
                    "tools": {}
                }
            })
        }
        "tools/list" => {
            json!({
                "tools": [
                    {
                        "name": "load_openapi",
                        "description": "Load an OpenAPI/YAML spec file",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "path": {"type": "string", "description": "Absolute path to an OpenAPI YAML file"}
                            },
                            "required": ["path"]
                        }
                    },
                    {
                        "name": "list_apis",
                        "description": "List all API endpoints",
                        "inputSchema": {"type": "object", "properties": {}}
                    },
                    {
                        "name": "get_api",
                        "description": "Get details for a specific API",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "path": {"type": "string"},
                                "method": {"type": "string", "enum": ["GET", "POST", "PUT", "DELETE", "PATCH"]}
                            },
                            "required": ["path", "method"]
                        }
                    },
                    {
                        "name": "search_apis",
                        "description": "Search APIs",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "keyword": {"type": "string"}
                            },
                            "required": ["keyword"]
                        }
                    },
                    {
                        "name": "get_servers",
                        "description": "Get server information",
                        "inputSchema": {"type": "object", "properties": {}}
                    }
                ]
            })
        }
        "tools/call" => {
            if let Some(params) = req.params.as_object() {
                let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let arguments = params.get("arguments").and_then(|v| v.as_object());

                let result = match tool_name {
                    "load_openapi" => {
                        if let Some(args) = arguments {
                            if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
                                match handler.load_spec(Path::new(path)).await {
                                    Ok(_) => json!({"content": [{"type": "text", "text": "Loaded successfully"}]}),
                                    Err(e) => json!({"content": [{"type": "text", "text": format!("Error: {}", e)}], "isError": true}),
                                }
                            } else {
                                json!({"content": [{"type": "text", "text": "Missing path argument"}], "isError": true})
                            }
                        } else {
                            json!({"content": [{"type": "text", "text": "Missing arguments"}], "isError": true})
                        }
                    }
                    "list_apis" => {
                        match handler.list_apis().await {
                            Ok(r) => json!({"content": [{"type": "text", "text": r}]}),
                            Err(e) => json!({"content": [{"type": "text", "text": format!("Error: {}", e)}], "isError": true}),
                        }
                    }
                    "get_api" => {
                        if let Some(args) = arguments {
                            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
                            let method = args.get("method").and_then(|v| v.as_str()).unwrap_or("");
                            match handler.get_api(path, method).await {
                                Ok(r) => json!({"content": [{"type": "text", "text": r}]}),
                                Err(e) => json!({"content": [{"type": "text", "text": format!("Error: {}", e)}], "isError": true}),
                            }
                        } else {
                            json!({"content": [{"type": "text", "text": "Missing arguments"}], "isError": true})
                        }
                    }
                    "search_apis" => {
                        if let Some(args) = arguments {
                            let keyword = args.get("keyword").and_then(|v| v.as_str()).unwrap_or("");
                            match handler.search_apis(keyword).await {
                                Ok(r) => json!({"content": [{"type": "text", "text": r}]}),
                                Err(e) => json!({"content": [{"type": "text", "text": format!("Error: {}", e)}], "isError": true}),
                            }
                        } else {
                            json!({"content": [{"type": "text", "text": "Missing arguments"}], "isError": true})
                        }
                    }
                    "get_servers" => {
                        match handler.get_servers().await {
                            Ok(r) => json!({"content": [{"type": "text", "text": r}]}),
                            Err(e) => json!({"content": [{"type": "text", "text": format!("Error: {}", e)}], "isError": true}),
                        }
                    }
                    _ => json!({"content": [{"type": "text", "text": format!("Unknown tool: {}", tool_name)}], "isError": true}),
                };
                result
            } else {
                json!({"content": [{"type": "text", "text": "Missing arguments"}], "isError": true})
            }
        }
        _ => json!({
            "error": {"code": -32601, "message": format!("Unknown method: {}", req.method)}
        }),
    };

    JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id: req.id,
        result: Some(result),
        error: None,
    }
}
