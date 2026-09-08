use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use nano_cli::{
    run, run_workflow, CliError, ErrorKind, Output, ValidationErrorKind, WorkflowRunOptions,
};
use nano_core::{BranchSchema, BranchSpec, BranchType};
use nano_io::read_events;
use nano_io::writer::{write_events, OutputBranch};
use nano_producers::{MuonProducer, MuonSkimRow};
use nano_rootio::write::{write_tree, Branch};

fn repo_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

#[test]
fn validate_muon_toml_reports_resolved_summary() {
    let spec = repo_path("crates/nano-spec/examples/muon.toml");
    let output = run(["validate", spec.to_str().unwrap()]).expect("validate command");

    let Output::Validate(report) = output else {
        panic!("expected validate report");
    };

    assert_eq!(report.analysis.name, "muon_demo");
    assert_eq!(report.analysis.objects[0].name, "good_muon");
    assert_eq!(report.analysis.regions, vec!["signal"]);
    assert_eq!(
        report
            .read_branches
            .iter()
            .map(|branch| (branch.name.as_str(), branch.branch_type.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("nMuon", "U32"),
            ("Muon_eta", "VecF32"),
            ("Muon_pt", "VecF32")
        ]
    );
}

#[test]
fn validate_broken_spec_fails_with_structured_errors() {
    let spec = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/broken-muon.toml");
    let error = run(["--json", "validate", spec.to_str().unwrap()])
        .expect_err("broken spec should fail validation");

    assert_eq!(error.kind, ErrorKind::Validation);
    assert_validation_kind(&error, ValidationErrorKind::MissingUnit);
    assert_validation_kind(&error, ValidationErrorKind::MissingBranch);
    assert_validation_kind(&error, ValidationErrorKind::UndefinedObject);

    let json = nano_cli::render_json_error(&error).expect("JSON error");
    assert!(json.contains("\"kind\": \"validation\""));
    assert!(json.contains("\"kind\": \"missing_unit\""));
    assert!(json.contains("\"branch\": \"Muon_nope\""));
    assert!(json.contains("\"object\": \"ghost_muon\""));
}

#[test]
fn branches_muon_toml_reports_derived_read_branches() {
    let spec = repo_path("crates/nano-spec/examples/muon.toml");
    let output = run(["branches", spec.to_str().unwrap()]).expect("branches command");

    let Output::Branches(report) = output else {
        panic!("expected branches report");
    };

    assert_eq!(
        report
            .branches
            .iter()
            .map(|branch| format!("{} {}", branch.name, branch.branch_type))
            .collect::<Vec<_>>(),
        vec!["nMuon U32", "Muon_eta VecF32", "Muon_pt VecF32"]
    );

    let json = nano_cli::render_json_output(&Output::Branches(report)).expect("JSON branches");
    assert!(json.contains("\"command\": \"branches\""));
    assert!(json.contains("\"name\": \"Muon_pt\""));
}

#[test]
fn certify_muon_toml_reports_certificate_json() {
    let spec = repo_path("crates/nano-spec/examples/muon.toml");
    let output = run(["certify", spec.to_str().unwrap()]).expect("certify command");

    let Output::Certify(report) = output else {
        panic!("expected certify report");
    };

    assert_eq!(report.certificate.analysis, "muon_demo");
    assert!(report
        .certificate
        .required_branches
        .iter()
        .any(|branch| branch.name == "Muon_pt"));
    assert_eq!(report.certificate.hash.len(), 16);

    let text = nano_cli::render_text(&Output::Certify(report));
    let json = serde_json::from_str::<serde_json::Value>(&text).expect("certificate JSON");
    assert_eq!(json["analysis"], "muon_demo");
    assert!(json["hash"].as_str().is_some());
}

#[test]
fn validate_and_branches_report_models() {
    let spec = repo_path("crates/nano-spec/examples/muon_tagger.toml");
    let output = run(["validate", spec.to_str().unwrap()]).expect("validate command");

    let Output::Validate(report) = output else {
        panic!("expected validate report");
    };

    assert_eq!(report.analysis.models.len(), 1);
    assert_eq!(report.analysis.models[0].name, "muon_tagger");
    assert_eq!(report.analysis.models[0].output, "Muon_topscore");

    let output = run(["branches", spec.to_str().unwrap()]).expect("branches command");
    let Output::Branches(report) = output else {
        panic!("expected branches report");
    };

    assert_eq!(report.models[0].provider, "Mock");
    assert!(report
        .branches
        .iter()
        .any(|branch| branch.name == "Muon_phi"));
}

#[test]
fn codegen_muon_toml_emits_generated_producer_source() {
    let spec = repo_path("crates/nano-spec/examples/muon.toml");
    let output = run(["codegen", spec.to_str().unwrap()]).expect("codegen command");

    let Output::Codegen(report) = output else {
        panic!("expected codegen report");
    };

    assert!(report
        .source
        .contains("// @generated by nano-spec from analysis `muon_demo`"));
    assert!(report.source.contains("pub struct GenRow"));
    assert!(report.source.contains("pub lead_muon_pt: f32"));
    assert!(report.source.contains("event.collection(\"Muon\")?"));
    assert!(report
        .source
        .contains("impl nano_analysis::Region for SignalRegion"));
    assert!(report
        .source
        .contains("baseline.select::<SignalRegion>(|_| n_good_muon >= 1_u32)"));
}

#[test]
fn codegen_muon_tagger_toml_emits_inference_producer_source() {
    let spec = repo_path("crates/nano-spec/examples/muon_tagger.toml");
    let output = run(["codegen", spec.to_str().unwrap()]).expect("codegen command");

    let Output::Codegen(report) = output else {
        panic!("expected codegen report");
    };

    assert!(report
        .source
        .contains("impl nano_analysis::ModelTag for MuonTagger"));
    assert!(report
        .source
        .contains("predictor: &impl nano_inference::Predictor"));
    assert!(report
        .source
        .contains("muon_tagger_baseline.infer::<MuonTagger>"));
    assert!(report
        .source
        .contains("muon_tagger_baseline.select::<SignalRegion>"));
}

#[test]
fn inspect_bundled_root_file_lists_ttrees() {
    let root_file = repo_path("crates/root-io/src/test_data/simple.root");
    let output = run(["inspect", root_file.to_str().unwrap()]).expect("inspect command");

    let Output::Inspect(report) = output else {
        panic!("expected inspect report");
    };

    assert!(report
        .trees
        .iter()
        .any(|tree| tree.name == "tree" && tree.entries > 0));

    let json = nano_cli::render_json_output(&Output::Inspect(report)).expect("JSON inspect");
    assert!(json.contains("\"command\": \"inspect\""));
    assert!(json.contains("\"name\": \"tree\""));
}

#[cfg(not(feature = "http"))]
#[test]
fn inspect_url_without_http_feature_reports_rebuild_hint() {
    let error = run(["inspect", "https://example.invalid/file.root"]).expect_err("inspect error");

    assert_eq!(error.kind, nano_cli::ErrorKind::Inspect);
    assert!(error.message.contains("requires HTTP support"));
    assert!(error.message.contains("--features http"));
}

#[test]
fn compare_identical_root_files_reports_pass() {
    let fixture = Fixture::new("compare-pass");
    let reference = fixture.path("reference.root");
    let candidate = fixture.path("candidate.root");
    write_compare_file(&reference, vec![1.0, 2.0, 3.0]);
    write_compare_file(&candidate, vec![1.0, 2.0, 3.0]);

    let output = run([
        "compare",
        reference.to_str().unwrap(),
        candidate.to_str().unwrap(),
    ])
    .expect("compare command");

    let Output::Compare(report) = output else {
        panic!("expected compare report");
    };
    assert!(report.passed());
    assert!(report
        .branches
        .iter()
        .all(|branch| branch.n_mismatched == 0));
}

#[test]
fn compare_mismatch_json_is_well_formed_and_binary_exits_nonzero() {
    let fixture = Fixture::new("compare-fail");
    let reference = fixture.path("reference.root");
    let candidate = fixture.path("candidate.root");
    write_compare_file(&reference, vec![1.0, 2.0, 3.0]);
    write_compare_file(&candidate, vec![1.0, 2.2, 3.0]);

    let output = run([
        "--json",
        "compare",
        reference.to_str().unwrap(),
        candidate.to_str().unwrap(),
    ])
    .expect("compare command returns structured report");
    let json = nano_cli::render_json_output(&output).expect("JSON compare");
    let value = serde_json::from_str::<serde_json::Value>(&json).expect("well-formed JSON");
    assert_eq!(value["command"], "compare");
    assert_eq!(value["status"], "fail");
    let pt = value["branches"]
        .as_array()
        .unwrap()
        .iter()
        .find(|branch| branch["name"] == "pt")
        .expect("pt branch");
    assert_eq!(pt["n_mismatched"], 1);

    let binary = std::process::Command::new(env!("CARGO_BIN_EXE_nano"))
        .arg("--json")
        .arg("compare")
        .arg(&reference)
        .arg(&candidate)
        .output()
        .expect("run nano binary");
    assert!(!binary.status.success());
    let stdout = String::from_utf8(binary.stdout).expect("stdout utf8");
    assert!(stdout.contains("\"command\": \"compare\""));
    assert!(stdout.contains("\"status\": \"fail\""));
}

#[test]
fn run_muon_spec_writes_skim_matching_single_pass_producer() {
    let fixture = Fixture::new("run-muon");
    let input = fixture.path("input.root");
    let output = fixture.path("skim.root");
    write_synthetic_input(&input);

    let report = run_workflow(WorkflowRunOptions {
        spec_path: repo_path("crates/nano-spec/examples/muon.toml"),
        inputs: vec![input.clone()],
        output: Some(output.clone()),
        parallel: false,
        kernel: None,
        interpret: false,
    })
    .expect("run workflow");

    assert_eq!(report.mode, "compiled");
    assert_eq!(report.kernel, "muon");
    assert_eq!(report.events_seen, 5);
    assert_eq!(report.events_selected, 3);
    assert_eq!(read_skim_rows(&output), single_pass_rows(&input));
    assert!(report.manifest.as_ref().expect("manifest path").exists());
}

#[test]
fn run_json_output_is_well_formed() {
    let fixture = Fixture::new("run-json");
    let input = fixture.path("input.root");
    let output = fixture.path("skim.root");
    write_synthetic_input(&input);

    let output = run([
        "--json",
        "run",
        repo_path("crates/nano-spec/examples/muon.toml")
            .to_str()
            .unwrap(),
        "--inputs",
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
    ])
    .expect("run command");

    let json = nano_cli::render_json_output(&output).expect("JSON run");
    let value = serde_json::from_str::<serde_json::Value>(&json).expect("well-formed JSON");
    assert_eq!(value["command"], "run");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["mode"], "compiled");
    assert_eq!(value["kernel"], "muon");
    assert_eq!(value["events_selected"], 3);
}

#[test]
fn run_interpret_writes_same_skim_as_compiled_kernel() {
    let fixture = Fixture::new("run-interpret-cross-backend");
    let input = fixture.path("input.root");
    let compiled_output = fixture.path("compiled.root");
    let interpreted_output = fixture.path("interpreted.root");
    write_synthetic_input(&input);

    let compiled = run_workflow(WorkflowRunOptions {
        spec_path: repo_path("crates/nano-spec/examples/muon.toml"),
        inputs: vec![input.clone()],
        output: Some(compiled_output.clone()),
        parallel: false,
        kernel: None,
        interpret: false,
    })
    .expect("compiled run");
    let interpreted = run([
        "run",
        "--interpret",
        repo_path("crates/nano-spec/examples/muon.toml")
            .to_str()
            .unwrap(),
        "--inputs",
        input.to_str().unwrap(),
        "--output",
        interpreted_output.to_str().unwrap(),
    ])
    .expect("interpreted run");

    let Output::Run(interpreted) = interpreted else {
        panic!("expected run report");
    };

    assert_eq!(compiled.mode, "compiled");
    assert_eq!(interpreted.mode, "interpret");
    assert_eq!(compiled.events_seen, interpreted.events_seen);
    assert_eq!(compiled.events_selected, interpreted.events_selected);
    assert_eq!(
        read_skim_rows(&compiled_output),
        read_skim_rows(&interpreted_output)
    );
}

#[test]
fn run_interpret_json_output_is_well_formed() {
    let fixture = Fixture::new("run-interpret-json");
    let input = fixture.path("input.root");
    write_synthetic_input(&input);

    let output = run([
        "--json",
        "run",
        "--interpret",
        repo_path("crates/nano-spec/examples/muon.toml")
            .to_str()
            .unwrap(),
        "--inputs",
        input.to_str().unwrap(),
    ])
    .expect("interpreted run command");

    let json = nano_cli::render_json_output(&output).expect("JSON run");
    let value = serde_json::from_str::<serde_json::Value>(&json).expect("well-formed JSON");
    assert_eq!(value["command"], "run");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["mode"], "interpret");
    assert_eq!(value["kernel"], "interpret");
    assert_eq!(value["events_seen"], 5);
    assert_eq!(value["events_selected"], 3);
    assert!(value.get("output").is_none());
}

#[test]
fn run_interpret_model_spec_returns_structured_unsupported_error() {
    let fixture = Fixture::new("run-interpret-model");
    let input = fixture.path("input.root");
    let error = run([
        "--json",
        "run",
        "--interpret",
        repo_path("crates/nano-spec/examples/muon_tagger.toml")
            .to_str()
            .unwrap(),
        "--inputs",
        input.to_str().unwrap(),
    ])
    .expect_err("model specs are not interpreted yet");

    assert_eq!(error.kind, ErrorKind::Interpret);
    assert!(error
        .message
        .contains("models not yet interpreted; use the compiled path"));

    let json = nano_cli::render_json_error(&error).expect("JSON error");
    let value = serde_json::from_str::<serde_json::Value>(&json).expect("well-formed JSON");
    assert_eq!(value["status"], "error");
    assert_eq!(value["kind"], "interpret");
    assert!(value["message"]
        .as_str()
        .unwrap()
        .contains("models not yet interpreted"));
}

#[test]
fn run_spec_without_registered_kernel_returns_structured_error() {
    let fixture = Fixture::new("run-no-kernel");
    let spec = fixture.path("electron_demo.toml");
    std::fs::write(
        &spec,
        include_str!("../../nano-spec/examples/muon.toml").replace("muon_demo", "electron_demo"),
    )
    .unwrap();

    let error = run_workflow(WorkflowRunOptions {
        spec_path: spec,
        inputs: vec![fixture.path("input.root")],
        output: Some(fixture.path("skim.root")),
        parallel: false,
        kernel: None,
        interpret: false,
    })
    .expect_err("spec should not resolve to a registered kernel");

    assert_eq!(error.kind, ErrorKind::Kernel);
    assert!(error
        .message
        .contains("no compiled kernel for spec `electron_demo`"));
    assert!(error
        .message
        .contains("codegen produces source to compile in"));
}

#[test]
fn run_muon_like_spec_with_incompatible_schema_returns_structured_error() {
    let fixture = Fixture::new("run-incompatible-kernel");
    let error = run_workflow(WorkflowRunOptions {
        spec_path: repo_path("crates/nano-spec/examples/muon_tagger.toml"),
        inputs: vec![fixture.path("input.root")],
        output: Some(fixture.path("skim.root")),
        parallel: false,
        kernel: None,
        interpret: false,
    })
    .expect_err("muon_tagger spec should not match the registered muon kernel");

    assert_eq!(error.kind, ErrorKind::Kernel);
    assert!(error
        .message
        .contains("not compatible with registered kernel `muon`"));
    assert!(error.message.contains("read_branches differ"));
}

#[test]
fn run_serial_and_parallel_outputs_are_identical() {
    let fixture = Fixture::new("run-serial-parallel");
    let input = fixture.path("input.root");
    let serial_output = fixture.path("serial.root");
    let parallel_output = fixture.path("parallel.root");
    write_synthetic_input(&input);

    let serial = run_workflow(WorkflowRunOptions {
        spec_path: repo_path("crates/nano-spec/examples/muon.toml"),
        inputs: vec![input.clone()],
        output: Some(serial_output.clone()),
        parallel: false,
        kernel: None,
        interpret: false,
    })
    .expect("serial run");
    let parallel = run_workflow(WorkflowRunOptions {
        spec_path: repo_path("crates/nano-spec/examples/muon.toml"),
        inputs: vec![input],
        output: Some(parallel_output.clone()),
        parallel: true,
        kernel: None,
        interpret: false,
    })
    .expect("parallel run");

    assert_eq!(serial.events_seen, parallel.events_seen);
    assert_eq!(serial.events_selected, parallel.events_selected);
    assert_eq!(
        read_skim_rows(&serial_output),
        read_skim_rows(&parallel_output)
    );
}

fn assert_validation_kind(error: &CliError, kind: ValidationErrorKind) {
    assert!(
        error
            .validation_errors
            .iter()
            .any(|validation_error| validation_error.kind == kind),
        "missing {kind:?} in {error:#?}"
    );
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

/// Read a fanned-out skim as `(nano_entry, n_good_muon, lead_muon_pt)` triples.
fn read_variation_rows(path: &Path) -> Vec<(u64, u32, f32)> {
    let schema = BranchSchema::new(vec![
        BranchSpec::new("nano_entry", BranchType::U64),
        BranchSpec::new("n_good_muon", BranchType::U32),
        BranchSpec::new("lead_muon_pt", BranchType::F32),
    ])
    .unwrap();
    read_events(path, schema)
        .unwrap()
        .iter()
        .map(|event| {
            (
                event.scalar::<u64>("nano_entry").unwrap(),
                event.scalar::<u32>("n_good_muon").unwrap(),
                event.scalar::<f32>("lead_muon_pt").unwrap(),
            )
        })
        .collect()
}

fn write_synthetic_input(path: &Path) {
    write_events(
        path,
        &[
            OutputBranch::u32("nMuon", vec![2, 1, 2, 0, 1]),
            OutputBranch::vec_f32(
                "Muon_pt",
                vec![
                    vec![31.0, 10.0],
                    vec![29.9],
                    vec![45.0, 35.0],
                    vec![],
                    vec![60.0],
                ],
            ),
            OutputBranch::vec_f32(
                "Muon_eta",
                vec![
                    vec![0.1, 0.2],
                    vec![0.0],
                    vec![2.39, -2.0],
                    vec![],
                    vec![2.39],
                ],
            ),
        ],
    )
    .unwrap();
    assert_eq!(read_events(path, input_schema()).unwrap().len(), 5);
}

fn write_compare_file(path: &Path, pt: Vec<f32>) {
    write_tree(
        path,
        "Events",
        &[
            Branch::u64("event", vec![10, 11, 12]),
            Branch::f32("pt", pt),
        ],
    )
    .unwrap();
}

fn single_pass_rows(path: &Path) -> Vec<MuonSkimRow> {
    read_events(path, input_schema())
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
            "nano-cli-{}-{timestamp}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

/// A shape systematic moves events across the region boundary, so each variation
/// must get its own skim: `Muon_pt > 30 GeV` under a ±5% JES shift admits event 1
/// only when varied up and drops event 0 when varied down. A single nominal run
/// would report neither migration.
#[test]
fn run_interpret_fans_the_skim_out_over_shape_variations() {
    let fixture = Fixture::new("run-interpret-shape-fanout");
    let input = fixture.path("input.root");
    let output = fixture.path("skim.root");
    write_synthetic_input(&input);

    let report = run([
        "run",
        "--interpret",
        repo_path("crates/nano-spec/examples/muon_hist_shape_correction.toml")
            .to_str()
            .unwrap(),
        "--inputs",
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
    ])
    .expect("interpreted run");

    let Output::Run(report) = report else {
        panic!("expected run report");
    };

    let variations = report.variations.expect("shape spec fans out");
    assert_eq!(
        variations
            .iter()
            .map(|variation| variation.systematic.as_str())
            .collect::<Vec<_>>(),
        vec!["Nominal", "JesUp", "JesDown"],
    );
    // The nominal skim keeps the requested path; the others are suffixed.
    assert_eq!(variations[0].output.as_deref(), Some(output.as_path()));
    assert_eq!(
        variations[1].output.as_deref(),
        Some(fixture.path("skim__JesUp.root").as_path())
    );

    // Each variation selects its own events, and `nano_entry` says which.
    let nominal = read_variation_rows(&output);
    let up = read_variation_rows(&fixture.path("skim__JesUp.root"));
    let down = read_variation_rows(&fixture.path("skim__JesDown.root"));

    let entries = |rows: &[(u64, u32, f32)]| rows.iter().map(|row| row.0).collect::<Vec<_>>();
    assert_eq!(entries(&nominal), vec![0, 2, 4]);
    assert_eq!(entries(&up), vec![0, 1, 2, 4], "event 1 migrates in when varied up");
    assert_eq!(entries(&down), vec![2, 4], "event 0 migrates out when varied down");

    // The report's headline figures stay nominal, and every variation is listed.
    assert_eq!(report.events_seen, 5);
    assert_eq!(report.events_selected, 3);
    assert_eq!(
        variations
            .iter()
            .map(|variation| variation.events_selected)
            .collect::<Vec<_>>(),
        vec![3, 4, 2],
    );

    // The join key is what makes the separate skims comparable: entry 2 is the
    // same input event in all three, with its pt shifted by the variation.
    let pt_at = |rows: &[(u64, u32, f32)], entry: u64| {
        rows.iter().find(|row| row.0 == entry).unwrap().2
    };
    assert!(pt_at(&up, 2) > pt_at(&nominal, 2));
    assert!(pt_at(&down, 2) < pt_at(&nominal, 2));
}

/// A spec with no declared variation is untouched by the fan-out: one file, the
/// old schema, no `nano_entry` column, no `variations` in the report.
#[test]
fn run_interpret_leaves_a_nominal_only_spec_unchanged() {
    let fixture = Fixture::new("run-interpret-nominal-only");
    let input = fixture.path("input.root");
    let output = fixture.path("skim.root");
    write_synthetic_input(&input);

    let report = run([
        "run",
        "--interpret",
        repo_path("crates/nano-spec/examples/muon.toml")
            .to_str()
            .unwrap(),
        "--inputs",
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
    ])
    .expect("interpreted run");

    let Output::Run(report) = report else {
        panic!("expected run report");
    };
    assert!(report.variations.is_none());
    assert_eq!(report.events_selected, 3);
    assert!(!fixture.path("skim__JesUp.root").exists());

    // The skim still reads back under the original schema, with no join column.
    assert_eq!(read_skim_rows(&output).len(), 3);
    let branches = nano_rootio::RootFile::open(&output)
        .unwrap()
        .tree("Events")
        .unwrap()
        .branches()
        .into_iter()
        .map(|branch| branch.name)
        .collect::<Vec<_>>();
    assert!(
        !branches.iter().any(|name| name == "nano_entry"),
        "nominal-only skim gained a join column: {branches:?}"
    );
}
