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
fn validate_wz_vbs_accepts_explicit_nanoaod_v15_catalogue() {
    let spec = repo_path("crates/nano-spec/examples/wz_vbs.toml");
    let output = run([
        "validate",
        "--catalogue-version",
        "v15",
        spec.to_str().unwrap(),
    ])
    .expect("validate command");

    let Output::Validate(report) = output else {
        panic!("expected validate report");
    };

    assert_eq!(report.analysis.name, "wz_vbs_nominal");
    assert_eq!(report.catalogue_version, "v15");
    assert!(report
        .read_branches
        .iter()
        .any(|branch| { branch.name == "Electron_cutBased" && branch.branch_type == "VecU8" }));
}

#[test]
fn campaign_wz_vbs_declares_demo_and_validation_gates() {
    let campaign = repo_path("configs/validation/wz_vbs_2024_v15_campaign.toml");
    let output = run(["campaign", campaign.to_str().unwrap()]).expect("campaign command");

    let Output::Campaign(report) = output else {
        panic!("expected campaign report");
    };
    assert_eq!(report.name, "wz_vbs_2024_v15_campaign");
    assert_eq!(report.catalogue_version, "v15");
    assert!(report.demo.is_some());
    assert!(report
        .gates
        .iter()
        .any(|gate| gate.name == "legacy_wz_root_parity" && gate.status == "implemented"));
    assert!(report.gates.iter().any(|gate| {
        gate.scope == "branch_new_analysis"
            && gate.status == "implemented"
            && gate.kind == "yield_closure"
            && gate.expected_entries == Some(1)
    }));
}

#[test]
fn campaign_yield_closure_checks_materialized_root_entries() {
    let fixture = Fixture::new("campaign-yield");
    let artifact = fixture.path("skim.root");
    let campaign = fixture.path("campaign.toml");
    let spec = repo_path("crates/nano-spec/examples/muon.toml");
    write_compare_file(&artifact, vec![1.0, 2.0, 3.0]);
    std::fs::write(
        &campaign,
        format!(
            r#"
[campaign]
name = "yield_campaign"
analysis_spec = "{}"
catalogue_version = "v9"
purpose = "test materialized yield closure"

[[gate]]
name = "selected_entries"
kind = "yield_closure"
status = "implemented"
scope = "branch_new_analysis"
description = "check selected entries"
artifact = "skim.root"
tree = "Events"
expected_entries = 3
"#,
            spec.display()
        ),
    )
    .unwrap();

    let output = run(["campaign", campaign.to_str().unwrap()]).expect("campaign command");

    let Output::Campaign(report) = output else {
        panic!("expected campaign report");
    };
    let gate = report
        .gates
        .iter()
        .find(|gate| gate.name == "selected_entries")
        .expect("yield gate");
    assert_eq!(gate.check_status, "passed");
    assert_eq!(gate.observed_entries, Some(3));
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
        .contains("let signal_predicate = n_good_muon >= 1_u32;"));
    assert!(report
        .source
        .contains("baseline.select::<SignalRegion>(|_| signal_predicate)"));
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
fn compare_branch_tolerance_flag_allows_named_branch_jitter() {
    let fixture = Fixture::new("compare-branch-tolerance");
    let reference = fixture.path("reference.root");
    let candidate = fixture.path("candidate.root");
    write_compare_file(&reference, vec![1.0, 2.0, 3.0]);
    write_compare_file(&candidate, vec![1.0, 2.2, 3.0]);

    let output = run([
        "compare",
        reference.to_str().unwrap(),
        candidate.to_str().unwrap(),
        "--rtol",
        "0",
        "--atol",
        "0",
        "--branch-tolerance",
        "pt:0:0.25",
    ])
    .expect("compare command");

    let Output::Compare(report) = output else {
        panic!("expected compare report");
    };
    assert!(report.passed());
    assert_eq!(report.branch_tolerances.len(), 1);
    assert_eq!(
        report
            .branches
            .iter()
            .find(|branch| branch.name == "pt")
            .unwrap()
            .n_mismatched,
        0
    );
}

#[test]
fn compare_validation_spec_supplies_branch_tolerance_policy() {
    let fixture = Fixture::new("compare-validation-spec");
    let reference = fixture.path("reference.root");
    let candidate = fixture.path("candidate.root");
    let spec = fixture.path("validation.toml");
    write_compare_file(&reference, vec![1.0, 2.0, 3.0]);
    write_compare_file(&candidate, vec![1.0, 2.2, 3.0]);
    std::fs::write(
        &spec,
        r#"
[analysis]
name = "validation_policy"
year = "Run2024"

[validation.compare]
rtol = 0.0
atol = 0.0

[[validation.compare.branch_tolerance]]
branch = "pt"
rtol = 0.0
atol = 0.25
reason = "test stochastic branch"
"#,
    )
    .unwrap();

    let output = run([
        "compare",
        reference.to_str().unwrap(),
        candidate.to_str().unwrap(),
        "--validation-spec",
        spec.to_str().unwrap(),
    ])
    .expect("compare command");

    let Output::Compare(report) = output else {
        panic!("expected compare report");
    };
    assert!(report.passed());
    assert_eq!(report.tolerance.rtol, 0.0);
    assert_eq!(report.tolerance.atol, 0.0);
    assert_eq!(report.branch_tolerances.len(), 1);
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
        max_events: None,
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
fn run_input_list_executes_direct_sources_without_intermediate_skim() {
    let fixture = Fixture::new("run-input-list");
    let input_a = fixture.path("input-a.root");
    let input_b = fixture.path("input-b.root");
    let input_list = fixture.path("direct-sources.txt");
    let output = fixture.path("skim.root");
    write_synthetic_input(&input_a);
    write_synthetic_input(&input_b);
    std::fs::write(
        &input_list,
        format!(
            "# Direct raw NanoAOD sources\n{}\n\n{}\n",
            input_a.display(),
            input_b.display()
        ),
    )
    .unwrap();

    let output_report = run([
        "run",
        repo_path("crates/nano-spec/examples/muon.toml")
            .to_str()
            .unwrap(),
        "--input-list",
        input_list.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
    ])
    .expect("run command");

    let Output::Run(report) = output_report else {
        panic!("expected run report");
    };
    let expected_rows = single_pass_rows(&input_a)
        .into_iter()
        .chain(single_pass_rows(&input_b))
        .collect::<Vec<_>>();

    assert_eq!(report.mode, "compiled");
    assert_eq!(report.kernel, "muon");
    assert_eq!(report.inputs, vec![input_a, input_b]);
    assert_eq!(report.events_seen, 10);
    assert_eq!(report.events_selected, 6);
    assert_eq!(read_skim_rows(&output), expected_rows);
}

#[test]
fn eos_sources_resolves_sample_yaml_to_direct_input_list() {
    let fixture = Fixture::new("eos-sources");
    let store = fixture.path("store");
    let sample_yaml = fixture.path("samples.yaml");
    let output = fixture.path("sources.txt");
    let mc_file = store
        .join("mc")
        .join("RunIII2024Summer24NanoAODv15")
        .join("WZJJto3LNu-EWK_TuneCP5_13p6TeV_madgraph-pythia8")
        .join("NANOAODSIM")
        .join("150X_mcRun3_2024_realistic_v2-v2")
        .join("0000")
        .join("mc.root");
    let data_file = store
        .join("data")
        .join("Run2024C")
        .join("Muon0")
        .join("NANOAOD")
        .join("MINIv6NANOv15-v1")
        .join("2530000")
        .join("data.root");
    std::fs::create_dir_all(mc_file.parent().unwrap()).unwrap();
    std::fs::create_dir_all(data_file.parent().unwrap()).unwrap();
    std::fs::write(&mc_file, "").unwrap();
    std::fs::write(&data_file, "").unwrap();
    std::fs::write(
        &sample_yaml,
        "\
wz_vbs_ewk:
- /WZJJto3LNu-EWK_TuneCP5_13p6TeV_madgraph-pythia8/RunIII2024Summer24NanoAODv15-150X_mcRun3_2024_realistic_v2-v2/NANOAODSIM
data_check:
- [/Muon0/Run2024C-MINIv6NANOv15-v1/NANOAOD]
",
    )
    .unwrap();

    let output_report = run([
        "eos-sources",
        sample_yaml.to_str().unwrap(),
        "--store-root",
        store.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
        "--max-files-per-dataset",
        "1",
    ])
    .expect("eos-sources command");

    let Output::EosSources(report) = output_report else {
        panic!("expected eos-sources report");
    };
    let source_list = std::fs::read_to_string(&output).unwrap();

    assert_eq!(report.files, 2);
    assert!(source_list.contains("# sample: wz_vbs_ewk"));
    assert!(source_list.contains(&mc_file.display().to_string()));
    assert!(source_list.contains(&data_file.display().to_string()));
}

#[test]
fn run_empty_input_list_returns_usage_error() {
    let fixture = Fixture::new("run-empty-input-list");
    let input_list = fixture.path("empty.txt");
    std::fs::write(&input_list, "# no inputs yet\n\n").unwrap();

    let error = run([
        "run",
        repo_path("crates/nano-spec/examples/muon.toml")
            .to_str()
            .unwrap(),
        "--input-list",
        input_list.to_str().unwrap(),
    ])
    .expect_err("empty input list should fail");

    assert_eq!(error.kind, ErrorKind::Usage);
    assert!(error.message.contains("did not contain any input sources"));
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
        max_events: None,
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
fn run_interpret_honors_max_events() {
    let fixture = Fixture::new("run-interpret-max-events");
    let input = fixture.path("input.root");
    write_synthetic_input(&input);

    let output = run([
        "run",
        "--interpret",
        "--max-events",
        "2",
        repo_path("crates/nano-spec/examples/muon.toml")
            .to_str()
            .unwrap(),
        "--inputs",
        input.to_str().unwrap(),
    ])
    .expect("interpreted run");

    let Output::Run(report) = output else {
        panic!("expected run report");
    };

    assert_eq!(report.events_seen, 2);
    assert_eq!(report.events_selected, 1);
}

#[test]
fn run_interpret_union_spec_writes_channel_index() {
    let fixture = Fixture::new("run-interpret-union");
    let spec = fixture.path("union.toml");
    let input = fixture.path("input.root");
    let output = fixture.path("union.root");
    write_synthetic_input(&input);
    std::fs::write(
        &spec,
        r#"
[analysis]
name = "union_demo"
year = "Run2018"

[[channel]]
name = "high"

[channel.objects.good_muon]
source = "Muon"
cuts = ["pt > 50 GeV", "abs(eta) < 2.4"]

[channel.regions.signal]
require = ["count(good_muon) >= 1"]

[[channel.outputs]]
name = "lead_muon_pt"
expr = "leading(good_muon).pt"

[[channel]]
name = "loose"

[channel.objects.good_muon]
source = "Muon"
cuts = ["pt > 30 GeV", "abs(eta) < 2.4"]

[channel.regions.signal]
require = ["count(good_muon) >= 1"]

[[channel.outputs]]
name = "lead_muon_pt"
expr = "leading(good_muon).pt"
"#,
    )
    .unwrap();

    let run_output = run([
        "run",
        "--interpret",
        spec.to_str().unwrap(),
        "--inputs",
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
    ])
    .expect("interpreted union run");

    let Output::Run(report) = run_output else {
        panic!("expected run report");
    };
    let rows = read_events(
        &output,
        BranchSchema::new([
            BranchSpec::new("channel_index", BranchType::U32),
            BranchSpec::new("lead_muon_pt", BranchType::F32),
        ])
        .unwrap(),
    )
    .unwrap();
    let channel_indices = rows
        .iter()
        .map(|event| event.scalar::<u32>("channel_index").unwrap())
        .collect::<Vec<_>>();

    assert_eq!(report.events_seen, 5);
    assert_eq!(report.events_selected, 4);
    assert_eq!(channel_indices, vec![1, 1, 0, 1]);
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
        max_events: None,
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
        max_events: None,
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
        max_events: None,
    })
    .expect("serial run");
    let parallel = run_workflow(WorkflowRunOptions {
        spec_path: repo_path("crates/nano-spec/examples/muon.toml"),
        inputs: vec![input],
        output: Some(parallel_output.clone()),
        parallel: true,
        kernel: None,
        interpret: false,
        max_events: None,
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
