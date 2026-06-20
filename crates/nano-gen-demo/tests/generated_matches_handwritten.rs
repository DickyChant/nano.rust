use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use nano_core::{BranchColumn, BranchSchema, BranchSpec, BranchType, Event};
use nano_gen_demo::GeneratedProducer;
use nano_io::writer::{write_events, OutputBranch};
use nano_producers::{MuonProducer, MuonSkimRow};
use nano_workflow::{
    merge_partials, muon_schema, plan_workflow_with_kernel_id, run_chunk, ExecutionMode, Executor,
    KernelRegistry, RunChunkRequest,
};

#[test]
fn generated_muon_producer_matches_handwritten_producer_on_synthetic_events() {
    for entry in 0..5 {
        let event = synthetic_event(entry);

        let generated = GeneratedProducer::analyze(&event)
            .unwrap()
            .map(|row| (row.n_good_muon, row.lead_muon_pt));
        let handwritten = MuonProducer::analyze(&event)
            .unwrap()
            .map(|row| (row.n_good_muon, row.lead_muon_pt));

        assert_eq!(generated, handwritten, "entry {entry}");
    }
}

#[test]
fn workflow_executes_generated_muon_kernel_like_handwritten_kernel() {
    let fixture = Fixture::new("generated-workflow");
    let input = fixture.path("input.root");
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

    let schema = muon_schema();
    let handwritten_plan = plan_workflow_with_kernel_id(
        [&input],
        schema.clone(),
        2,
        fixture.path("cache-handwritten"),
        fixture.path("handwritten.root"),
        MuonProducer::analyze,
        "muon",
    )
    .unwrap();
    let generated_plan = plan_workflow_with_kernel_id(
        [&input],
        schema.clone(),
        2,
        fixture.path("cache-generated"),
        fixture.path("generated.root"),
        generated_muon_as_skim,
        "generated_muon",
    )
    .unwrap();

    let executor = Executor::new();
    let handwritten = executor
        .run(&handwritten_plan, ExecutionMode::Serial)
        .unwrap()
        .merged;
    let generated = executor
        .run(&generated_plan, ExecutionMode::Serial)
        .unwrap()
        .merged;
    assert_eq!(generated, handwritten);

    let mut registry = KernelRegistry::with_muon();
    registry.register("generated_muon", schema, generated_muon_as_skim);
    let partials = generated_plan
        .maps
        .iter()
        .map(|map| {
            run_chunk(
                &RunChunkRequest {
                    source: map.chunk.source.clone(),
                    entry_range: map.chunk.entry_range.clone(),
                    kernel_id: "generated_muon".to_string(),
                },
                &registry,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(merge_partials(partials), handwritten);
}

fn generated_muon_as_skim(event: &Event) -> nano_core::Result<Option<MuonSkimRow>> {
    GeneratedProducer::analyze(event).map(|row| {
        row.map(|row| MuonSkimRow {
            n_good_muon: row.n_good_muon,
            lead_muon_pt: row.lead_muon_pt,
        })
    })
}

fn synthetic_event(entry: usize) -> Event {
    Event::from_columns(schema(), columns(), entry).unwrap()
}

fn schema() -> BranchSchema {
    BranchSchema::new([
        BranchSpec::new("nMuon", BranchType::U32),
        BranchSpec::new("Muon_pt", BranchType::VecF32),
        BranchSpec::new("Muon_eta", BranchType::VecF32),
    ])
    .unwrap()
}

fn columns() -> Vec<(String, BranchColumn)> {
    vec![
        ("nMuon".to_string(), BranchColumn::U32(vec![2, 1, 2, 0, 1])),
        (
            "Muon_pt".to_string(),
            BranchColumn::VecF32(vec![
                vec![31.0, 10.0],
                vec![29.9],
                vec![45.0, 35.0],
                vec![],
                vec![60.0],
            ]),
        ),
        (
            "Muon_eta".to_string(),
            BranchColumn::VecF32(vec![
                vec![0.1, 0.2],
                vec![0.0],
                vec![2.39, -2.0],
                vec![],
                vec![2.39],
            ]),
        ),
    ]
}

fn write_synthetic_input(path: &Path, muons: Vec<Vec<(f32, f32)>>) {
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
            "nano-gen-demo-{}-{timestamp}-{name}",
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
