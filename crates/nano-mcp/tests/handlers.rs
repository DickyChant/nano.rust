use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use nano_core::{BranchSchema, BranchSpec, BranchType};
use nano_io::read_events;
use nano_io::writer::{write_events, OutputBranch};
use nano_mcp::{
    derive_read_branches, generate_kernel, handle_json_rpc, inspect_file, run_workflow,
    validate_spec, InspectFileInput, RunWorkflowInput, SpecInput, ToolErrorKind,
    ValidationErrorKind,
};
use serde_json::{json, Value};

fn repo_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

#[test]
fn validate_spec_muon_toml_returns_structured_success() {
    let result = validate_spec(SpecInput {
        spec_path: Some(repo_path("crates/nano-spec/examples/muon.toml")),
        spec_text: None,
        format: None,
    });

    assert!(result.ok);
    assert_eq!(result.analysis.expect("analysis").name, "muon_demo");
    assert_eq!(result.objects[0].name, "good_muon");
    assert_eq!(result.regions, vec!["signal"]);
    assert_eq!(result.outputs, vec!["n_good_muon", "lead_muon_pt"]);
    assert!(result.errors.is_empty());
}

#[test]
fn validate_spec_broken_text_returns_structured_validation_errors() {
    let result = validate_spec(SpecInput {
        spec_path: None,
        spec_text: Some(include_str!("../../nano-cli/tests/fixtures/broken-muon.toml").to_string()),
        format: None,
    });

    assert!(!result.ok);
    assert_eq!(result.errors.len(), 1);
    assert_eq!(result.errors[0].kind, ToolErrorKind::Validation);

    let validation_kinds = result.errors[0]
        .validation_errors
        .iter()
        .map(|error| error.kind)
        .collect::<Vec<_>>();
    assert!(validation_kinds.contains(&ValidationErrorKind::MissingUnit));
    assert!(validation_kinds.contains(&ValidationErrorKind::MissingBranch));
    assert!(validation_kinds.contains(&ValidationErrorKind::UndefinedObject));
}

#[test]
fn derive_read_branches_muon_toml_returns_branch_schema() {
    let result = derive_read_branches(SpecInput {
        spec_path: Some(repo_path("crates/nano-spec/examples/muon.toml")),
        spec_text: None,
        format: None,
    });

    assert!(result.ok);
    assert_eq!(
        result
            .branches
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
fn generate_kernel_muon_toml_returns_source() {
    let result = generate_kernel(SpecInput {
        spec_path: Some(repo_path("crates/nano-spec/examples/muon.toml")),
        spec_text: None,
        format: None,
    });

    assert!(result.ok);
    let source = result.source.expect("source");
    assert!(source.contains("pub struct GenRow"));
    assert!(source.contains("pub lead_muon_pt: f32"));
}

#[test]
fn inspect_file_bundled_root_file_lists_trees() {
    let result = inspect_file(InspectFileInput {
        path: repo_path("crates/root-io/src/test_data/simple.root"),
    });

    assert!(result.ok);
    assert!(result
        .trees
        .iter()
        .any(|tree| tree.name == "tree" && tree.entries > 0));
}

#[test]
fn run_workflow_muon_spec_returns_structured_success() {
    let fixture = Fixture::new("mcp-run");
    let input = fixture.path("input.root");
    let output = fixture.path("skim.root");
    write_synthetic_input(&input);

    let result = run_workflow(RunWorkflowInput {
        spec_path: repo_path("crates/nano-spec/examples/muon.toml"),
        inputs: vec![input],
        output: Some(output.clone()),
        parallel: false,
    });

    assert!(result.ok);
    assert_eq!(result.command.as_deref(), Some("run"));
    assert_eq!(result.status.as_deref(), Some("ok"));
    assert_eq!(result.kernel.as_deref(), Some("muon"));
    assert_eq!(result.events_seen, Some(5));
    assert_eq!(result.events_selected, Some(3));
    assert_eq!(result.output.as_ref(), Some(&output));
    assert!(result.manifest.expect("manifest").exists());
    assert_eq!(read_events(&output, skim_schema()).unwrap().len(), 3);
    assert!(result.errors.is_empty());
}

#[test]
fn json_rpc_tools_list_and_call_shape_match_mcp() {
    let initialize = handle_json_rpc(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    }))
    .expect("initialize response");
    assert_eq!(initialize["result"]["serverInfo"]["name"], "nano-mcp");

    let list = handle_json_rpc(json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {}
    }))
    .expect("tools/list response");
    let names = list["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        vec![
            "validate_spec",
            "derive_read_branches",
            "inspect_file",
            "generate_kernel",
            "run_workflow"
        ]
    );

    let call = handle_json_rpc(json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "validate_spec",
            "arguments": {
                "spec_path": repo_path("crates/nano-spec/examples/muon.toml")
            }
        }
    }))
    .expect("tools/call response");
    assert_eq!(call["result"]["structuredContent"]["ok"], true);
    assert_eq!(
        call["result"]["structuredContent"]["analysis"]["name"],
        "muon_demo"
    );
    assert_eq!(call["result"]["isError"], false);
}

#[test]
fn stdio_round_trip_lists_tools_and_validates_muon_toml() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_nano-mcp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn nano-mcp");

    {
        let stdin = child.stdin.as_mut().expect("stdin");
        writeln!(
            stdin,
            "{}",
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}})
        )
        .unwrap();
        writeln!(
            stdin,
            "{}",
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}})
        )
        .unwrap();
        writeln!(
            stdin,
            "{}",
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "validate_spec",
                    "arguments": {
                        "spec_path": repo_path("crates/nano-spec/examples/muon.toml")
                    }
                }
            })
        )
        .unwrap();
    }

    let output = child.wait_with_output().expect("wait for nano-mcp");
    assert!(output.status.success());

    let responses = String::from_utf8(output.stdout)
        .expect("utf8 stdout")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("json response"))
        .collect::<Vec<_>>();
    assert_eq!(responses.len(), 3);
    assert_eq!(responses[0]["result"]["serverInfo"]["name"], "nano-mcp");
    assert!(responses[1]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["name"] == "run_workflow"));
    assert_eq!(responses[2]["result"]["structuredContent"]["ok"], true);
    assert_eq!(
        responses[2]["result"]["structuredContent"]["analysis"]["name"],
        "muon_demo"
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
            "nano-mcp-{}-{timestamp}-{name}",
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
