# supp mcp

Start an MCP (Model Context Protocol) server over stdio, allowing AI assistants like Claude Code to call supp tools directly.

## Usage

```
supp mcp
```

The server communicates via JSON-RPC over stdin/stdout. It's not meant to be run manually — register it with your AI tool and it will be started automatically.

## Registration

### Project-scoped (`.mcp.json` in project root)

```json
{
  "mcpServers": {
    "supp": {
      "type": "stdio",
      "command": "supp",
      "args": ["mcp"]
    }
  }
}
```

### Global (`~/.claude/settings.json`)

```json
{
  "mcpServers": {
    "supp": {
      "type": "stdio",
      "command": "supp",
      "args": ["mcp"]
    }
  }
}
```

## Tools

| Tool | Description | Key params |
|------|-------------|------------|
| `supp_diff` | Compare git changes | `path?`, `cached?`, `untracked?`, `local?`, `branch?`, `all?`, `self_branch?`, `context_lines?`, `filter?` |
| `supp_ctx` | Single-file analysis with deps and usage | `file`, `mode?` |
| `supp_why` | Deep-dive a symbol | `query` |
| `supp_sym` | Search symbols by name | `query` |
| `supp_tree` | Directory tree with git status | `path?`, `depth?`, `no_git?` |
| `supp_context` | Multi-file/directory context | `paths[]`, `depth?`, `mode?`, `regex?` |

The `mode` parameter accepts `"full"` (default), `"slim"`, or `"map"`.

## Testing

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0.1"}}}' | supp mcp
```

## Notes

- The `pick` command is excluded — it requires an interactive terminal
- All tools run synchronously under the hood via `spawn_blocking`
- Config from `~/.supp/config.toml` is respected for defaults (context lines, depth, etc.)
