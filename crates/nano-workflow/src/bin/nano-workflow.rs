use std::path::{Path, PathBuf};
use std::process::ExitCode;

use nano_workflow::{
    export_portable_graph, import_portable_graph, merge_partial_files, muon_schema,
    plan_muon_workflow, run_chunk_to_path, write_merged_output, write_muon_skim, ExecutionMode,
    Executor, KernelRegistry, PortableGraph, RunChunkRequest,
};

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let (command, rest) = args.split_first().ok_or_else(|| usage("missing command"))?;
    match command.as_str() {
        "export" => export_command(rest),
        "run-chunk" => run_chunk_command(rest),
        "merge" => merge_command(rest),
        "run-local" => run_local_command(rest),
        _ => Err(usage(format!("unknown command `{command}`"))),
    }
}

fn export_command(args: &[String]) -> Result<(), String> {
    let output = option_value(args, "-o")?.ok_or_else(|| usage("export requires -o graph.json"))?;
    let chunk_size = option_value(args, "--chunk-size")?
        .map(|value| parse_usize(value, "--chunk-size"))
        .transpose()?
        .unwrap_or(65_536);
    let inputs = positional_args(args, &["-o", "--chunk-size"])?;
    if inputs.is_empty() {
        return Err(usage("export requires at least one input"));
    }

    let graph_path = PathBuf::from(output);
    let graph_parent = graph_path.parent().unwrap_or_else(|| Path::new("."));
    let cache_dir = graph_parent.join("nano-workflow-cache");
    let skim_path = graph_parent.join("muon-skim.root");
    let input_paths = inputs.iter().map(PathBuf::from).collect::<Vec<_>>();
    let plan = plan_muon_workflow(
        &input_paths,
        muon_schema(),
        chunk_size,
        cache_dir,
        skim_path,
    )
    .map_err(|error| error.to_string())?;
    let graph = export_portable_graph(&plan);
    write_json(&graph_path, &graph)?;
    Ok(())
}

fn run_chunk_command(args: &[String]) -> Result<(), String> {
    let source = option_value(args, "--source")?
        .ok_or_else(|| usage("run-chunk requires --source <file-or-url>"))?;
    let start = option_value(args, "--start")?
        .ok_or_else(|| usage("run-chunk requires --start <i>"))
        .and_then(|value| parse_usize(value, "--start"))?;
    let stop = option_value(args, "--stop")?
        .ok_or_else(|| usage("run-chunk requires --stop <j>"))
        .and_then(|value| parse_usize(value, "--stop"))?;
    let kernel = option_value(args, "--kernel")?.unwrap_or("muon");
    let output =
        option_value(args, "-o")?.ok_or_else(|| usage("run-chunk requires -o partial.json"))?;
    if stop < start {
        return Err(usage("--stop must be greater than or equal to --start"));
    }

    let request = RunChunkRequest::new(source, start, stop, kernel);
    run_chunk_to_path(&request, output, &KernelRegistry::with_muon())
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn merge_command(args: &[String]) -> Result<(), String> {
    let output = option_value(args, "-o")?.ok_or_else(|| usage("merge requires -o merged.json"))?;
    let skim = option_value(args, "--skim")?;
    let partials = positional_args(args, &["-o", "--skim"])?;
    if partials.is_empty() {
        return Err(usage("merge requires at least one partial.json input"));
    }

    let merged = merge_partial_files(partials.iter().map(PathBuf::from))
        .map_err(|error| error.to_string())?;
    write_merged_output(output, &merged).map_err(|error| error.to_string())?;
    if let Some(skim_path) = skim {
        write_muon_skim(Path::new(skim_path), &merged).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn run_local_command(args: &[String]) -> Result<(), String> {
    let parallel = args.iter().any(|arg| arg == "--parallel");
    let graph_path = args
        .iter()
        .find(|arg| arg.as_str() != "--parallel")
        .ok_or_else(|| usage("run-local requires graph.json"))?;
    let graph = read_json::<PortableGraph>(Path::new(graph_path))?;
    let plan = import_portable_graph(&graph).map_err(|error| error.to_string())?;
    let mode = if parallel {
        ExecutionMode::Parallel
    } else {
        ExecutionMode::Serial
    };
    let report = Executor::new()
        .run(&plan, mode)
        .map_err(|error| error.to_string())?;
    let mode_label = match report.mode {
        ExecutionMode::Serial => "serial",
        ExecutionMode::Parallel => "parallel",
    };
    println!(
        "mode={mode_label} maps_executed={} maps_skipped={} reduce_executed={} sink_executed={} rows={}",
        report.maps.executed,
        report.maps.skipped,
        report.reduce.executed,
        report.sink.executed,
        report.merged.rows.len()
    );
    Ok(())
}

fn option_value<'a>(args: &'a [String], option: &str) -> Result<Option<&'a str>, String> {
    let mut index = 0;
    while index < args.len() {
        if args[index] == option {
            return args
                .get(index + 1)
                .map(String::as_str)
                .ok_or_else(|| usage(format!("{option} requires a value")))
                .map(Some);
        }
        index += 1;
    }
    Ok(None)
}

fn positional_args<'a>(
    args: &'a [String],
    valued_options: &[&str],
) -> Result<Vec<&'a str>, String> {
    let mut values = Vec::new();
    let mut index = 0;
    while index < args.len() {
        if valued_options.contains(&args[index].as_str()) {
            if index + 1 >= args.len() {
                return Err(usage(format!("{} requires a value", args[index])));
            }
            index += 2;
        } else if args[index].starts_with('-') {
            return Err(usage(format!("unknown option `{}`", args[index])));
        } else {
            values.push(args[index].as_str());
            index += 1;
        }
    }
    Ok(values)
}

fn parse_usize(value: &str, label: &str) -> Result<usize, String> {
    value
        .parse()
        .map_err(|_| usage(format!("{label} must be a non-negative integer")))
}

fn write_json<T>(path: &Path, value: &T) -> Result<(), String>
where
    T: serde::Serialize,
{
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    std::fs::write(path, bytes).map_err(|error| error.to_string())
}

fn read_json<T>(path: &Path) -> Result<T, String>
where
    T: for<'de> serde::Deserialize<'de>,
{
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

fn usage(message: impl AsRef<str>) -> String {
    format!(
        "{}\nusage:\n  nano-workflow export <inputs...> -o graph.json [--chunk-size n]\n  nano-workflow run-chunk --source <f> --start <i> --stop <j> [--kernel muon] -o partial.json\n  nano-workflow merge <partial.json...> -o merged.json [--skim skim.root]\n  nano-workflow run-local <graph.json> [--parallel]",
        message.as_ref()
    )
}
