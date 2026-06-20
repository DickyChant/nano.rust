use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use nano_core::{BranchSchema, BranchSpec, BranchType};
use nano_io::read_events;
use nano_io::writer::{write_events, OutputBranch};
use nano_producers::{MuonProducer, MuonSkimRow};
use nano_workflow::{plan_muon_workflow, ExecutionMode, Executor, RunStats};

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
