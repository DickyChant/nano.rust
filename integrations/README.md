# nano-workflow boundary adapters

`graph.json` is the portable workflow IR. Dask and Ray do not run analysis code
in Python here; they only schedule shell commands that invoke the Rust
`nano-workflow` task unit.

Build the binary first:

```bash
cargo build -p nano-workflow
export PATH="$PWD/target/debug:$PATH"
```

Export a graph for local ROOT inputs:

```bash
nano-workflow export input-a.root input-b.root -o graph.json --chunk-size 50000
```

Run with Dask:

```python
from integrations.dask_runner import run_graph

merged_json = run_graph("graph.json", binary="nano-workflow")
print(merged_json)
```

Run with Ray:

```python
import ray
from integrations.ray_runner import run_graph

ray.init(address="auto")  # or ray.init() for local testing
merged_json = run_graph("graph.json", binary="nano-workflow")
print(merged_json)
```

Both adapters submit one `nano-workflow run-chunk` command per map node, then a
single `nano-workflow merge ... --skim ...` reduce command. Python dependencies
are intentionally optional and are not required for `cargo test`.
