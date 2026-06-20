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
fn parses_real_jme_payload_when_present() {
    let path = Path::new("../../data/jme-derived/Run2-2018-UL-NanoAODv9/latest/jet_jerc.json.gz");
    if path.exists() {
        let set = CorrectionSet::from_path(path).unwrap();
        assert!(!set.corrections.is_empty());
    }
}
