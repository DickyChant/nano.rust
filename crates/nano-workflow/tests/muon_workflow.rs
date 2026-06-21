use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use nano_core::{BranchSchema, BranchSpec, BranchType};
use nano_io::read_events;
use nano_io::writer::{write_events, OutputBranch};
use nano_producers::{MuonProducer, MuonSkimRow};
use nano_workflow::{
    export_portable_graph, import_portable_graph, merge_partials, plan_muon_workflow, run_chunk,
    ExecutionMode, Executor, KernelRegistry, PortableGraph, RunChunkRequest, RunStats,
    WorkflowNodeKind,
};

#[test]
fn serial_and_parallel_outputs_match_single_pass() {
    let fixture = Fixture::new("match_single_pass");
    let input = fixture.path("input.root");
    let output = fixture.path("skim.root");
    write_synthetic_input(
        &input,
        vec![
            vec![(31.0, 0.1), (10.0, 0.2)],
            vec![(29.9, 0.0)],
            vec![(45.0, 2.39), (35.0, -2.0)],
            vec![],
            vec![(60.0, 2.39)],
        ],
    );

    let schema = input_schema();
    let serial_plan = plan_muon_workflow(
        [&input],
        schema.clone(),
        2,
        fixture.path("cache-serial"),
        &output,
    )
    .unwrap();
    let parallel_plan = plan_muon_workflow(
        [&input],
        schema.clone(),
        2,
        fixture.path("cache-parallel"),
        &output,
    )
    .unwrap();

    let report = Executor::new()
        .run_verified(&serial_plan, &parallel_plan)
        .unwrap();
    let expected_rows = single_pass_rows(&input, schema);

    assert_eq!(report.serial.mode, ExecutionMode::Serial);
    assert_eq!(report.parallel.mode, ExecutionMode::Parallel);
    assert_eq!(report.serial.merged, report.parallel.merged);
    assert_eq!(report.merged.rows, expected_rows);
    assert_eq!(
        report.merged.cutflow,
        nano_workflow::Cutflow {
            events_seen: 5,
            events_selected: 3,
        }
    );
    assert_eq!(read_skim_rows(&output), expected_rows);
}

#[test]
fn workflow_plan_node_summaries_report_kinds_and_chunk_count() {
    let fixture = Fixture::new("node_summaries");
    let input = fixture.path("input.root");
    let output = fixture.path("skim.root");
    write_synthetic_input(
        &input,
        vec![vec![(31.0, 0.1)], vec![(45.0, -0.3)], vec![(55.0, 0.4)]],
    );

    let plan = plan_muon_workflow([&input], input_schema(), 2, fixture.path("cache"), &output)
        .expect("workflow plans");
    let nodes = plan.node_summaries();

    assert_eq!(nodes.len(), 5);
    assert_eq!(
        nodes.iter().map(|node| node.kind).collect::<Vec<_>>(),
        vec![
            WorkflowNodeKind::Source,
            WorkflowNodeKind::Map,
            WorkflowNodeKind::Map,
            WorkflowNodeKind::Reduce,
            WorkflowNodeKind::Sink,
        ]
    );
    assert_eq!(
        nodes
            .iter()
            .filter(|node| node.kind == WorkflowNodeKind::Map)
            .count(),
        2
    );
}

#[test]
fn second_run_skips_all_nodes_and_preserves_output() {
    let fixture = Fixture::new("second_run_skips");
    let input = fixture.path("input.root");
    let output = fixture.path("skim.root");
    write_synthetic_input(
        &input,
        vec![
            vec![(31.0, 0.1)],
            vec![(29.0, 0.2)],
            vec![(42.0, -0.5), (12.0, 0.0)],
            vec![(55.0, 2.2)],
        ],
    );

    let schema = input_schema();
    let serial_plan = plan_muon_workflow(
        [&input],
        schema.clone(),
        2,
        fixture.path("cache-serial"),
        &output,
    )
    .unwrap();
    let parallel_plan =
        plan_muon_workflow([&input], schema, 2, fixture.path("cache-parallel"), &output).unwrap();
    let executor = Executor::new();

    let first = executor.run_verified(&serial_plan, &parallel_plan).unwrap();
    let second = executor.run_verified(&serial_plan, &parallel_plan).unwrap();

    assert_eq!(first.merged, second.merged);
    assert_eq!(second.serial.maps.executed, 0);
    assert_eq!(second.parallel.maps.executed, 0);
    assert_eq!(second.serial.maps.skipped, serial_plan.maps.len());
    assert_eq!(second.parallel.maps.skipped, parallel_plan.maps.len());
    assert_eq!(second.serial.reduce, skipped_one());
    assert_eq!(second.parallel.reduce, skipped_one());
    assert_eq!(second.serial.sink, skipped_one());
    assert_eq!(second.parallel.sink, skipped_one());
}

#[test]
fn changed_input_recomputes_only_affected_maps_then_reduce_and_sink() {
    let fixture = Fixture::new("changed_input");
    let input_a = fixture.path("input-a.root");
    let input_b = fixture.path("input-b.root");
    let output = fixture.path("skim.root");
    write_synthetic_input(&input_a, vec![vec![(31.0, 0.1)], vec![(20.0, 0.1)]]);
    write_synthetic_input(&input_b, vec![vec![(41.0, 0.1)], vec![(25.0, 0.1)]]);

    let schema = input_schema();
    let plan = plan_muon_workflow(
        [&input_a, &input_b],
        schema.clone(),
        2,
        fixture.path("cache"),
        &output,
    )
    .unwrap();
    let executor = Executor::new();
    let first = executor.run(&plan, ExecutionMode::Serial).unwrap();
    assert_eq!(first.maps.executed, 2);

    write_synthetic_input(
        &input_b,
        vec![vec![(41.0, 0.1), (33.0, 0.2)], vec![(66.0, 0.1)]],
    );
    let changed_plan = plan_muon_workflow(
        [&input_a, &input_b],
        schema.clone(),
        2,
        fixture.path("cache"),
        &output,
    )
    .unwrap();
    let changed = executor.run(&changed_plan, ExecutionMode::Serial).unwrap();
    let expected_rows = single_pass_rows(&input_a, schema.clone())
        .into_iter()
        .chain(single_pass_rows(&input_b, schema))
        .collect::<Vec<_>>();

    assert_eq!(changed.maps.executed, 1);
    assert_eq!(changed.maps.skipped, 1);
    assert_eq!(changed.reduce, executed_one());
    assert_eq!(changed.sink, executed_one());
    assert_eq!(changed.merged.rows, expected_rows);
}

#[test]
fn portable_export_import_round_trip_runs_like_in_memory_plan() {
    let fixture = Fixture::new("portable_round_trip");
    let input = fixture.path("input.root");
    write_synthetic_input(
        &input,
        vec![
            vec![(35.0, 0.1)],
            vec![(12.0, 0.2)],
            vec![(50.0, -1.1), (40.0, 2.0)],
            vec![],
        ],
    );

    let schema = input_schema();
    let in_memory_plan = plan_muon_workflow(
        [&input],
        schema.clone(),
        2,
        fixture.path("cache-memory"),
        fixture.path("memory.root"),
    )
    .unwrap();
    let portable_plan = plan_muon_workflow(
        [&input],
        schema,
        2,
        fixture.path("cache-portable"),
        fixture.path("portable.root"),
    )
    .unwrap();

    let graph = export_portable_graph(&portable_plan);
    let imported_plan = import_portable_graph(&graph).unwrap();
    assert_eq!(export_portable_graph(&imported_plan), graph);

    let executor = Executor::new();
    let expected = executor
        .run(&in_memory_plan, ExecutionMode::Serial)
        .unwrap()
        .merged;
    let imported = executor
        .run(&imported_plan, ExecutionMode::Serial)
        .unwrap()
        .merged;
    assert_eq!(imported, expected);
}

#[test]
fn task_atoms_match_single_pass_and_local_executor() {
    let fixture = Fixture::new("task_atoms");
    let input = fixture.path("input.root");
    write_synthetic_input(
        &input,
        vec![
            vec![(31.0, 0.1), (29.0, 0.1)],
            vec![(60.0, 2.5)],
            vec![(45.0, -0.3)],
            vec![(10.0, 0.1)],
            vec![(70.0, 1.1), (33.0, -2.0)],
        ],
    );

    let schema = input_schema();
    let plan = plan_muon_workflow(
        [&input],
        schema.clone(),
        2,
        fixture.path("cache"),
        fixture.path("skim.root"),
    )
    .unwrap();
    let registry = KernelRegistry::with_muon();
    let partials = plan
        .maps
        .iter()
        .map(|map| {
            run_chunk(
                &RunChunkRequest {
                    source: map.chunk.source.clone(),
                    entry_range: map.chunk.entry_range.clone(),
                    kernel_id: "muon".to_string(),
                },
                &registry,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let atom_merged = merge_partials(partials);
    let local_merged = Executor::new()
        .run(&plan, ExecutionMode::Serial)
        .unwrap()
        .merged;
    let single_pass = single_pass_rows(&input, schema);

    assert_eq!(atom_merged, local_merged);
    assert_eq!(atom_merged.rows, single_pass);
    assert_eq!(
        atom_merged.cutflow,
        nano_workflow::Cutflow {
            events_seen: 5,
            events_selected: 3,
        }
    );
}

#[test]
fn portable_graph_json_serializes_deserializes_stably() {
    let fixture = Fixture::new("portable_json");
    let input = fixture.path("input.root");
    write_synthetic_input(&input, vec![vec![(31.0, 0.1)], vec![(20.0, 0.1)]]);

    let plan = plan_muon_workflow(
        [&input],
        input_schema(),
        1,
        fixture.path("cache"),
        fixture.path("skim.root"),
    )
    .unwrap();
    let graph = export_portable_graph(&plan);
    let json = serde_json::to_string_pretty(&graph).unwrap();
    let decoded = serde_json::from_str::<PortableGraph>(&json).unwrap();
    let encoded_again = serde_json::to_string_pretty(&decoded).unwrap();

    assert_eq!(decoded, graph);
    assert_eq!(encoded_again, json);
    assert!(json.contains("\"schema_version\": 1"));
    assert!(json.contains("\"kind\": \"map\""));
    assert!(json.contains("\"kernel_id\": \"muon\""));
}

fn skipped_one() -> RunStats {
    RunStats {
        executed: 0,
        skipped: 1,
    }
}

fn executed_one() -> RunStats {
    RunStats {
        executed: 1,
        skipped: 0,
    }
}

fn input_schema() -> BranchSchema {
    BranchSchema::new([
        BranchSpec::new("nMuon", BranchType::U32),
        BranchSpec::new("Muon_pt", BranchType::VecF32),
        BranchSpec::new("Muon_eta", BranchType::VecF32),
    ])
    .unwrap()
}

fn skim_schema() -> BranchSchema {
    BranchSchema::new([
        BranchSpec::new("n_good_muon", BranchType::U32),
        BranchSpec::new("lead_muon_pt", BranchType::F32),
    ])
    .unwrap()
}

fn single_pass_rows(path: &Path, schema: BranchSchema) -> Vec<MuonSkimRow> {
    read_events(path, schema)
        .unwrap()
        .iter()
        .filter_map(|event| MuonProducer::analyze(event).unwrap())
        .collect()
}

fn read_skim_rows(path: &Path) -> Vec<MuonSkimRow> {
    read_events(path, skim_schema())
        .unwrap()
        .iter()
        .map(|event| MuonSkimRow {
            n_good_muon: event.scalar::<u32>("n_good_muon").unwrap(),
            lead_muon_pt: event.scalar::<f32>("lead_muon_pt").unwrap(),
        })
        .collect()
}

fn write_synthetic_input(path: &Path, muons: Vec<Vec<(f32, f32)>>) {
    let n_events = muons.len();
    let n_muon = muons
        .iter()
        .map(|event_muons| event_muons.len() as u32)
        .collect::<Vec<_>>();
    let muon_pt = muons
        .iter()
        .map(|event_muons| event_muons.iter().map(|(pt, _)| *pt).collect())
        .collect::<Vec<Vec<_>>>();
    let muon_eta = muons
        .iter()
        .map(|event_muons| event_muons.iter().map(|(_, eta)| *eta).collect())
        .collect::<Vec<Vec<_>>>();

    write_events(
        path,
        &[
            OutputBranch::u32("nMuon", n_muon),
            OutputBranch::vec_f32("Muon_pt", muon_pt),
            OutputBranch::vec_f32("Muon_eta", muon_eta),
        ],
    )
    .unwrap();
    assert_eq!(read_events(path, input_schema()).unwrap().len(), n_events);
}

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "nano-workflow-{}-{timestamp}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
