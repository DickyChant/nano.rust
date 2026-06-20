# Agent Interface

`nano-mcp` exposes nano.rust's verified operations as MCP tools over stdio. The
orchestrator proposes specs, file paths, or codegen requests; the nano.rust parser,
validator, ROOT reader, and code generator dispose of them with structured success or
structured error data.

The server is intentionally the agent-facing form of the same validation-confined action
space as the `nano` CLI. Tool effects are limited to parsing, validation, ROOT file
inspection, source generation, and running a validated workflow over inputs. A failed
validation is returned as data, not as a
transport error, so an agent can revise its next proposal from concrete diagnostics.

## Running

Build the server:

```bash
cargo build -p nano-mcp
```

Point an MCP client at the binary:

```json
{
  "mcpServers": {
    "nano": {
      "command": "/home/sqian/Work/nano.rust/target/debug/nano-mcp",
      "args": []
    }
  }
}
```

For Claude Desktop or Claude Code, use the same command path in the client's MCP server
configuration. The server speaks JSON-RPC 2.0 over stdio and supports `initialize`,
`tools/list`, and `tools/call`.

## Tools

`validate_spec`

Input:

```json
{ "spec_path": "crates/nano-spec/examples/muon.toml" }
```

or:

```json
{ "spec_text": "[analysis]\nname = \"muon_demo\"\nyear = \"Run2018\"\n", "format": "toml" }
```

Output:

```json
{
  "ok": true,
  "analysis": { "name": "muon_demo", "year": "Run2018" },
  "objects": [{ "name": "good_muon", "source": "Muon" }],
  "regions": ["signal"],
  "outputs": ["n_good_muon", "lead_muon_pt"],
  "errors": []
}
```

`derive_read_branches`

Input: `{ "spec_path": "...toml" }` or `{ "spec_text": "...", "format": "toml" }`.

Output:

```json
{
  "ok": true,
  "branches": [
    { "name": "nMuon", "type": "U32" },
    { "name": "Muon_eta", "type": "VecF32" },
    { "name": "Muon_pt", "type": "VecF32" }
  ],
  "errors": []
}
```

`inspect_file`

Input:

```json
{ "path": "path/to/file.root" }
```

Output:

```json
{
  "ok": true,
  "trees": [{ "name": "Events", "entries": 1000 }],
  "events_branches": [{ "name": "Muon_pt", "type": "Float_t" }],
  "errors": []
}
```

`generate_kernel`

Input: `{ "spec_path": "...toml" }` or `{ "spec_text": "...", "format": "toml" }`.

Output:

```json
{ "ok": true, "source": "generated Rust source...", "errors": [] }
```

`run_workflow`

Validate a spec, resolve a registered runtime kernel, execute the local workflow DAG
over ROOT inputs, and write the skim plus provenance manifest.

Input: `{ "spec_path": "...toml", "inputs": ["a.root", "b.root"], "output": "skim.root"?, "parallel": false? }`.

Output:

```json
{
  "ok": true,
  "inputs": ["a.root"],
  "events_seen": 1000,
  "events_selected": 348,
  "output": "skim.root",
  "errors": []
}
```

(Runtime execution uses the precompiled kernel registry; a spec with no compatible
compiled kernel returns `ok: false` with a `kernel` error — codegen produces source to
compile in, it is not JIT-run. See [`architecture.md`](architecture.md).)

## Errors

Domain failures are returned in the tool result with `ok: false` and `isError: true` at
the MCP tool-call layer:

```json
{
  "ok": false,
  "errors": [
    {
      "kind": "validation",
      "message": "spec validation failed",
      "validation_errors": [
        {
          "kind": "missing_branch",
          "message": "object `good_muon` cut 2: missing branch `Muon_nope`",
          "context": "object `good_muon` cut 2",
          "branch": "Muon_nope"
        }
      ]
    }
  ]
}
```

Error kinds are `usage`, `parse`, `catalogue`, `validation`, `codegen`, `inspect`,
and `kernel` (no compiled kernel for the spec, from `run_workflow`).
Validation diagnostics preserve typed fields such as `context`, `branch`, `object`,
`expr`, `expected`, `actual`, and `detail`.
