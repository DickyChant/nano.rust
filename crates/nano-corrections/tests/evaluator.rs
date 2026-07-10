use flate2::write::GzEncoder;
use flate2::Compression;
use nano_corrections::{CorrectionSet, MuonIdCorrection, MuonIdInput, Value, Variation, Year};
use std::fs::File;
use std::io::Write;
use std::path::Path;

const TEST_JSON: &str = r#"
{
  "schema_version": 2,
  "corrections": [
    {
      "name": "muon_id_sf",
      "description": "hand-written correctionlib fixture",
      "version": 1,
      "inputs": [
        {"name": "pt", "type": "real"},
        {"name": "eta", "type": "real"},
        {"name": "variation", "type": "string"}
      ],
      "output": {"name": "weight", "type": "real"},
      "data": {
        "nodetype": "category",
        "input": "variation",
        "content": [
          {
            "key": "nominal",
            "value": {
              "nodetype": "binning",
              "input": "eta",
              "edges": [0.0, 1.5, 2.5],
              "flow": "clamp",
              "content": [
                {
                  "nodetype": "formula",
                  "expression": "x[0] * param[0] + param[1]",
                  "parser": "TFormula",
                  "variables": ["pt"],
                  "parameters": [0.01, 1.0]
                },
                2.0
              ]
            }
          },
          {
            "key": "up",
            "value": {
              "nodetype": "binning",
              "input": "eta",
              "edges": [0.0, 1.5, 2.5],
              "flow": "clamp",
              "content": [1.6, 2.1]
            }
          },
          {
            "key": "down",
            "value": {
              "nodetype": "binning",
              "input": "eta",
              "edges": [0.0, 1.5, 2.5],
              "flow": "clamp",
              "content": [1.4, 1.9]
            }
          }
        ]
      }
    }
  ]
}
"#;

#[test]
fn evaluates_category_binning_and_formula() {
    let set = CorrectionSet::from_json_str(TEST_JSON).unwrap();
    let correction = set.correction("muon_id_sf").unwrap();

    let nominal = correction
        .evaluate(&[
            Value::Real(50.0),
            Value::Real(0.4),
            Value::Str("nominal".to_string()),
        ])
        .unwrap();
    assert_eq!(nominal, 1.5);

    let up = correction
        .evaluate(&[
            Value::Real(50.0),
            Value::Real(0.4),
            Value::Str("up".to_string()),
        ])
        .unwrap();
    assert_eq!(up, 1.6);

    let down = correction
        .evaluate(&[
            Value::Real(50.0),
            Value::Real(2.0),
            Value::Str("down".to_string()),
        ])
        .unwrap();
    assert_eq!(down, 1.9);
}

#[test]
fn evaluates_multibinning_with_flat_content_and_flow() {
    let set = CorrectionSet::from_json_str(
        r#"
{
  "schema_version": 2,
  "corrections": [
    {
      "name": "map2d",
      "version": 1,
      "inputs": [
        {"name": "x", "type": "real"},
        {"name": "y", "type": "real"}
      ],
      "output": {"name": "value", "type": "real"},
      "data": {
        "nodetype": "multibinning",
        "inputs": ["x", "y"],
        "edges": [[0.0, 1.0, 2.0], [0.0, 10.0, 20.0, 30.0]],
        "content": [1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        "flow": 0.0
      }
    }
  ]
}
"#,
    )
    .unwrap();
    let correction = set.correction("map2d").unwrap();

    assert_eq!(
        correction
            .evaluate(&[Value::Real(0.5), Value::Real(5.0)])
            .unwrap(),
        1.0
    );
    assert_eq!(
        correction
            .evaluate(&[Value::Real(0.5), Value::Real(15.0)])
            .unwrap(),
        2.0
    );
    assert_eq!(
        correction
            .evaluate(&[Value::Real(1.5), Value::Real(25.0)])
            .unwrap(),
        6.0
    );
    assert_eq!(
        correction
            .evaluate(&[Value::Real(-1.0), Value::Real(25.0)])
            .unwrap(),
        0.0
    );
}

#[test]
fn evaluates_compound_correction_stack_with_input_updates() {
    let set = CorrectionSet::from_json_str(
        r#"
{
  "schema_version": 2,
  "corrections": [
    {
      "name": "double",
      "version": 1,
      "inputs": [{"name": "x", "type": "real"}],
      "output": {"name": "factor", "type": "real"},
      "data": 2.0
    },
    {
      "name": "read_updated_x",
      "version": 1,
      "inputs": [{"name": "x", "type": "real"}],
      "output": {"name": "factor", "type": "real"},
      "data": {
        "nodetype": "formula",
        "expression": "x[0]",
        "parser": "TFormula",
        "variables": ["x"],
        "parameters": []
      }
    }
  ],
  "compound_corrections": [
    {
      "name": "compound",
      "description": "multiply output and update x after each stack element",
      "inputs": [{"name": "x", "type": "real"}],
      "output": {"name": "factor", "type": "real"},
      "inputs_update": ["x"],
      "input_op": "*",
      "output_op": "*",
      "stack": ["double", "read_updated_x"]
    }
  ]
}
"#,
    )
    .unwrap();

    let compound = set.compound_correction("compound").unwrap();
    let factor = compound.evaluate(&set, &[Value::Real(5.0)]).unwrap();
    let via_ref = set
        .correction_ref("compound")
        .unwrap()
        .evaluate(&set, &[Value::Real(5.0)])
        .unwrap();

    assert_eq!(factor, 20.0);
    assert_eq!(via_ref, 20.0);
}

#[test]
fn evaluates_tformula_comparison_gates() {
    let set = CorrectionSet::from_json_str(
        r#"
{
  "schema_version": 2,
  "corrections": [
    {
      "name": "gate",
      "version": 1,
      "inputs": [{"name": "pt", "type": "real"}],
      "output": {"name": "factor", "type": "real"},
      "data": {
        "nodetype": "formula",
        "expression": "((x<[0])*([1]))+((x>=[0])*([2]))",
        "parser": "TFormula",
        "variables": ["pt"],
        "parameters": [20.0, 1.5, 2.5]
      }
    }
  ]
}
"#,
    )
    .unwrap();

    let correction = set.correction("gate").unwrap();
    assert_eq!(correction.evaluate(&[Value::Real(10.0)]).unwrap(), 1.5);
    assert_eq!(correction.evaluate(&[Value::Real(20.0)]).unwrap(), 2.5);
}

#[test]
fn typed_muon_id_wrapper_maps_fields_to_declared_inputs() {
    let set = CorrectionSet::from_json_str(TEST_JSON).unwrap();
    let typed = MuonIdCorrection::new(set.correction("muon_id_sf").unwrap().clone());

    let nominal = typed
        .evaluate(MuonIdInput {
            pt: 50.0,
            eta: 0.4,
            year: Year::Run2018,
            variation: Variation::Nominal,
        })
        .unwrap();
    assert_eq!(nominal, 1.5);

    let up = typed
        .evaluate(MuonIdInput {
            pt: 50.0,
            eta: 0.4,
            year: Year::Run2018,
            variation: Variation::Up,
        })
        .unwrap();
    assert_eq!(up, 1.6);
}

#[test]
fn reads_gzipped_json_payload() {
    let path = std::env::temp_dir().join(format!(
        "nano-corrections-test-{}.json.gz",
        std::process::id()
    ));
    {
        let file = File::create(&path).unwrap();
        let mut encoder = GzEncoder::new(file, Compression::default());
        encoder.write_all(TEST_JSON.as_bytes()).unwrap();
        encoder.finish().unwrap();
    }

    let set = CorrectionSet::from_path(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert_eq!(set.correction("muon_id_sf").unwrap().name, "muon_id_sf");
}

#[test]
fn loads_real_gzipped_jme_payload_and_evaluates_total_uncertainty() {
    let path =
        Path::new("../../data/jme-derived/Run2-2016postVFP-UL-NanoAODv9/latest/jet_jerc.json.gz");
    let set = CorrectionSet::from_path(path).unwrap();
    let names = set
        .corrections
        .iter()
        .map(|correction| correction.name.as_str())
        .collect::<Vec<_>>();

    assert!(names.contains(&"Summer19UL16_V7_MC_Total_AK4PFPuppi"));

    let correction = set
        .correction("Summer19UL16_V7_MC_Total_AK4PFPuppi")
        .unwrap();
    let factor = correction
        .evaluate(&[Value::Real(0.5), Value::Real(100.0)])
        .unwrap();

    assert!(factor.is_finite());
    assert_eq!(factor, 0.0108);
}
