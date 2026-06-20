use nano_core::{BranchColumn, BranchSchema, BranchSpec, BranchType, Event};
use nano_inference::{
    events_to_infer_request, FeatureScope, InferRequest, MockPredictor, Predictor, Tensor,
    TensorData,
};
use rayon::prelude::*;
use std::sync::Arc;

#[test]
fn mock_predictor_deterministic() {
    let predictor = MockPredictor::new("unit");
    let req = request_from_values("unit", vec![1.0, 2.0, 3.0, 4.0], 2, 2);
    let same = request_from_values("unit", vec![1.0, 2.0, 3.0, 4.0], 2, 2);
    let different = request_from_values("unit", vec![1.0, 2.0, 3.0, 5.0], 2, 2);

    let first = predictor.predict(&req).unwrap();
    let second = predictor.predict(&same).unwrap();
    let third = predictor.predict(&different).unwrap();

    assert_eq!(first, second);
    assert_ne!(first, third);
}

#[test]
fn feature_extraction() {
    let events = sample_events();
    let req = events_to_infer_request(
        "fatjet_tagger",
        &events,
        FeatureScope::object("FatJet"),
        &["pt", "eta", "jetId"],
    )
    .unwrap();

    assert_eq!(req.model, "fatjet_tagger");
    assert_eq!(req.inputs.len(), 1);
    assert_eq!(req.inputs[0].name, "features");
    assert_eq!(req.inputs[0].shape, vec![3, 3]);
    assert_eq!(
        req.inputs[0].data,
        TensorData::F32(vec![350.0, 0.5, 6.0, 240.0, -1.5, 2.0, 125.0, 0.1, 6.0])
    );
}

#[test]
fn parallel_equals_serial() {
    let predictor = MockPredictor::new("parallel");
    let rows = 4096usize;
    let cols = 6usize;
    let values = (0..rows * cols)
        .map(|i| ((i * 17 + 3) % 1000) as f32)
        .collect::<Vec<_>>();

    let full_batch = request_from_values("parallel", values.clone(), rows, cols);
    let serial_full = predictor.predict(&full_batch).unwrap();

    let serial_rows = values
        .chunks(cols)
        .flat_map(|row| predict_row(&predictor, row))
        .collect::<Vec<_>>();
    let parallel_rows = values
        .par_chunks(cols)
        .flat_map(|row| predict_row(&predictor, row))
        .collect::<Vec<_>>();

    assert_eq!(serial_rows, parallel_rows);
    assert_eq!(serial_full.outputs[0].data, TensorData::F32(parallel_rows));
}

fn predict_row(predictor: &MockPredictor, row: &[f32]) -> Vec<f32> {
    let response = predictor
        .predict(&request_from_values("parallel", row.to_vec(), 1, row.len()))
        .unwrap();
    match response.outputs.into_iter().next().unwrap().data {
        TensorData::F32(values) => values,
        _ => unreachable!("mock predictor emits f32"),
    }
}

fn request_from_values(model: &str, values: Vec<f32>, rows: usize, cols: usize) -> InferRequest {
    InferRequest {
        model: model.to_string(),
        inputs: vec![Tensor {
            name: "features".to_string(),
            shape: vec![rows, cols],
            data: TensorData::F32(values),
        }],
    }
}

fn sample_events() -> Vec<Event> {
    let schema = Arc::new(
        BranchSchema::new([
            BranchSpec::new("FatJet_pt", BranchType::VecF32),
            BranchSpec::new("FatJet_eta", BranchType::VecF32),
            BranchSpec::new("FatJet_jetId", BranchType::VecU8),
            BranchSpec::new("MET_pt", BranchType::F32),
        ])
        .unwrap(),
    );
    let columns = Arc::new(nano_core::EventColumns::from_ordered([
        (
            "FatJet_pt",
            BranchColumn::VecF32(vec![vec![350.0, 240.0], vec![125.0]]),
        ),
        (
            "FatJet_eta",
            BranchColumn::VecF32(vec![vec![0.5, -1.5], vec![0.1]]),
        ),
        (
            "FatJet_jetId",
            BranchColumn::VecU8(vec![vec![6, 2], vec![6]]),
        ),
        ("MET_pt", BranchColumn::F32(vec![80.0, 42.0])),
    ]));

    (0..2)
        .map(|row| {
            Event::from_shared_event_columns_at(Arc::clone(&schema), Arc::clone(&columns), row, row)
                .unwrap()
        })
        .collect()
}
