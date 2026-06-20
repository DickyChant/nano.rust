import json
import subprocess
from pathlib import Path

from dask import delayed


def run_graph(graph_path, binary="nano-workflow"):
    """Run a PortableGraph with Dask delayed tasks.

    The scheduler only decides placement. Each compute unit is the Rust
    nano-workflow binary.
    """
    graph_path = Path(graph_path)
    graph = json.loads(graph_path.read_text())
    maps = [_map_node(node, binary) for node in graph["nodes"] if node["kind"] == "map"]
    reduce_node = _single_node(graph, "reduce")
    sink_node = _single_node(graph, "sink")
    merged = delayed(_merge)(maps, reduce_node["output_path"], sink_node["output_path"], binary)
    return merged.compute()


def _map_node(node, binary):
    entry_range = node["entry_range"]
    return delayed(_run_chunk)(
        node["source"],
        entry_range["start"],
        entry_range["end"],
        node.get("kernel_id", "muon"),
        node["output_path"],
        binary,
    )


def _run_chunk(source, start, stop, kernel_id, output_path, binary):
    subprocess.run(
        [
            binary,
            "run-chunk",
            "--source",
            source,
            "--start",
            str(start),
            "--stop",
            str(stop),
            "--kernel",
            kernel_id,
            "-o",
            output_path,
        ],
        check=True,
    )
    return output_path


def _merge(partial_paths, output_path, skim_path, binary):
    subprocess.run(
        [binary, "merge", *partial_paths, "-o", output_path, "--skim", skim_path],
        check=True,
    )
    return output_path


def _single_node(graph, kind):
    matches = [node for node in graph["nodes"] if node["kind"] == kind]
    if len(matches) != 1:
        raise ValueError(f"expected exactly one {kind} node, got {len(matches)}")
    return matches[0]
