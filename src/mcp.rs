use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, tool, tool_handler, tool_router,
};

use crate::compress::Mode;
use crate::config::Config;
use crate::git::DiffOptions;

// ── Parameter structs ──────────────────────────────────────────

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DiffParams {
    #[schemars(description = "Path or registered repo name (defaults to '.')")]
    #[serde(default)]
    pub path: Option<String>,
    #[schemars(description = "Only diff untracked files")]
    #[serde(default)]
    pub untracked: bool,
    #[schemars(description = "Unstaged changes to tracked files only")]
    #[serde(default)]
    pub tracked: bool,
    #[schemars(description = "Staged changes only")]
    #[serde(default)]
    pub staged: bool,
    #[schemars(description = "All local changes vs self branch remote")]
    #[serde(default)]
    pub local: bool,
    #[schemars(description = "All branch changes vs remote default main (default)")]
    #[serde(default)]
    pub all: bool,
    #[schemars(description = "Branch to compare to (used with all)")]
    #[serde(default)]
    pub branch: Option<String>,
    #[schemars(description = "Number of context lines in unified diff")]
    #[serde(default)]
    pub context_lines: Option<u32>,
    #[schemars(description = "Regex pattern to filter file paths (e.g. '\\.rs$')")]
    #[serde(default)]
    pub regex: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CtxParams {
    #[schemars(description = "File path to analyze")]
    pub file: String,
    #[schemars(description = "Analysis mode: 'full', 'slim', or 'map' (default: 'full')")]
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct WhyParams {
    #[schemars(description = "Symbol name to look up (space-separated tokens)")]
    pub query: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SymParams {
    #[schemars(description = "Search query (space-separated tokens)")]
    pub query: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct TreeParams {
    #[schemars(description = "Directory path (defaults to '.')")]
    #[serde(default)]
    pub path: Option<String>,
    #[schemars(description = "Maximum depth to display")]
    #[serde(default)]
    pub depth: Option<usize>,
    #[schemars(description = "Disable git status indicators")]
    #[serde(default)]
    pub no_git: bool,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ContextParams {
    #[schemars(description = "Paths for context generation (files and/or directories)")]
    pub paths: Vec<String>,
    #[schemars(description = "Tree depth in context header (default: 2)")]
    #[serde(default)]
    pub depth: Option<usize>,
    #[schemars(description = "Analysis mode: 'full', 'slim', or 'map' (default: 'full')")]
    #[serde(default)]
    pub mode: Option<String>,
    #[schemars(description = "Regex pattern to filter file paths")]
    #[serde(default)]
    pub regex: Option<String>,
}

// ── Helpers ────────────────────────────────────────────────────

fn parse_mode(s: Option<&str>) -> Mode {
    match s {
        Some("slim") => Mode::Slim,
        Some("map") => Mode::Map,
        _ => Mode::Full,
    }
}

fn err(msg: String) -> McpError {
    McpError::internal_error(msg, None)
}

// ── Server ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SuppServer {
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl SuppServer {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "Compare git changes in a repository. Returns unified diff output.")]
    async fn supp_diff(
        &self,
        Parameters(params): Parameters<DiffParams>,
    ) -> Result<CallToolResult, McpError> {
        tokio::task::spawn_blocking(move || {
            let config = Config::load();
            let repo_path = params.path.as_deref().unwrap_or(".");
            let max_untracked_size = config.limits.max_untracked_file_size_mb * 1024 * 1024;
            let opts = DiffOptions {
                untracked: params.untracked,
                tracked: params.tracked,
                staged: params.staged,
                local: params.local,
                all: params.all,
                branch: params.branch,
                context_lines: params.context_lines.or(Some(config.diff.context_lines)),
                max_untracked_size,
            };
            let result = crate::git::get_diff(repo_path, opts, params.regex.as_deref())
                .map_err(|e| err(e.to_string()))?;
            Ok(CallToolResult::success(vec![Content::text(result.text)]))
        })
        .await
        .map_err(|e| err(e.to_string()))?
    }

    #[tool(
        description = "Analyze a single file: dependencies, usage, and full content with context."
    )]
    async fn supp_ctx(
        &self,
        Parameters(params): Parameters<CtxParams>,
    ) -> Result<CallToolResult, McpError> {
        tokio::task::spawn_blocking(move || {
            let mode = parse_mode(params.mode.as_deref());
            let files = vec![params.file];
            let result =
                crate::ctx::analyze(".", &files, 2, None, mode).map_err(|e| err(e.to_string()))?;
            Ok(CallToolResult::success(vec![Content::text(result.plain)]))
        })
        .await
        .map_err(|e| err(e.to_string()))?
    }

    #[tool(
        description = "Deep-dive a symbol: full definition, doc comments, call sites, and dependencies."
    )]
    async fn supp_why(
        &self,
        Parameters(params): Parameters<WhyParams>,
    ) -> Result<CallToolResult, McpError> {
        tokio::task::spawn_blocking(move || {
            let query: Vec<String> = params.query.split_whitespace().map(String::from).collect();
            let result = crate::why::explain(".", &query).map_err(|e| err(e.to_string()))?;
            Ok(CallToolResult::success(vec![Content::text(result.plain)]))
        })
        .await
        .map_err(|e| err(e.to_string()))?
    }

    #[tool(description = "Search symbols by name with PageRank-powered ranking.")]
    async fn supp_sym(
        &self,
        Parameters(params): Parameters<SymParams>,
    ) -> Result<CallToolResult, McpError> {
        tokio::task::spawn_blocking(move || {
            let query: Vec<String> = params.query.split_whitespace().map(String::from).collect();
            let result = crate::symbol::search(".", &query).map_err(|e| err(e.to_string()))?;

            let mut output = String::new();
            for (sym, score) in &result.matches {
                let parent = sym
                    .parent
                    .as_deref()
                    .map(|p| format!(" (in {p})"))
                    .unwrap_or_default();
                output.push_str(&format!(
                    "{:.1}  {:>12}  {}{}  {}:{}\n       {}\n\n",
                    score,
                    format!("{:?}", sym.kind),
                    sym.name,
                    parent,
                    sym.file,
                    sym.line,
                    sym.signature,
                ));
            }
            if output.is_empty() {
                output.push_str("No matching symbols found.\n");
            } else {
                output.push_str(&format!(
                    "({} of {} symbols)\n",
                    result.matches.len(),
                    result.total_symbols
                ));
            }
            Ok(CallToolResult::success(vec![Content::text(output)]))
        })
        .await
        .map_err(|e| err(e.to_string()))?
    }

    #[tool(description = "Display a directory tree with optional git status indicators.")]
    async fn supp_tree(
        &self,
        Parameters(params): Parameters<TreeParams>,
    ) -> Result<CallToolResult, McpError> {
        tokio::task::spawn_blocking(move || {
            let root = params.path.as_deref().unwrap_or(".");
            let statuses = if params.no_git {
                None
            } else {
                crate::git::get_status_map(root).map_err(|e| err(e.to_string()))?
            };
            let status_ref = statuses
                .as_ref()
                .map(|(map, prefix)| (map, prefix.as_str()));
            let result = crate::tree::build_tree(root, params.depth, None, status_ref)
                .map_err(|e| err(e.to_string()))?;
            Ok(CallToolResult::success(vec![Content::text(result.plain)]))
        })
        .await
        .map_err(|e| err(e.to_string()))?
    }

    #[tool(
        description = "Generate structured context for one or more files/directories with tree headers and content."
    )]
    async fn supp_context(
        &self,
        Parameters(params): Parameters<ContextParams>,
    ) -> Result<CallToolResult, McpError> {
        tokio::task::spawn_blocking(move || {
            let config = Config::load();
            let mode = parse_mode(params.mode.as_deref());
            let depth = params.depth.unwrap_or(config.global.depth);
            let result = crate::context::generate_context(
                &params.paths,
                depth,
                params.regex.as_deref(),
                mode,
            )
            .map_err(|e: anyhow::Error| err(e.to_string()))?;
            Ok(CallToolResult::success(vec![Content::text(result.plain)]))
        })
        .await
        .map_err(|e| err(e.to_string()))?
    }
}

#[tool_handler]
impl ServerHandler for SuppServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            instructions: Some("supp: structured code context for LLMs — diffs, file analysis, symbol search, tree views".to_string()),
            ..Default::default()
        }
    }
}

pub async fn run() -> anyhow::Result<()> {
    use rmcp::ServiceExt;
    let server = SuppServer::new().serve(rmcp::transport::stdio()).await?;
    server.waiting().await?;
    Ok(())
}
