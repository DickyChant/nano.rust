import json
import subprocess
from pathlib import Path

import ray


def submit_graph(graph_path, binary="nano-workflow"):
    """Submit a PortableGraph to Ray and return the reduce ObjectRef."""
    graph_path = Path(graph_path)
    graph = json.loads(graph_path.read_text())
    maps = [_run_map_node.remote(node, binary) for node in graph["nodes"] if node["kind"] == "map"]
    reduce_node = _single_node(graph, "reduce")
    sink_node = _single_node(graph, "sink")
    return _merge.remote(maps, reduce_node["output_path"], sink_node["output_path"], binary)


def run_graph(graph_path, binary="nano-workflow"):
    """Submit a PortableGraph to Ray and block until the Rust reduce finishes."""
    return ray.get(submit_graph(graph_path, binary=binary))


@ray.remote
def _run_map_node(node, binary):
    entry_range = node["entry_range"]
    subprocess.run(
        [
            binary,
            "run-chunk",
            "--source",
            node["source"],
            "--start",
            str(entry_range["start"]),
            "--stop",
            str(entry_range["end"]),
            "--kernel",
            node.get("kernel_id", "muon"),
            "-o",
            node["output_path"],
        ],
        check=True,
    )
    return node["output_path"]


@ray.remote
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
