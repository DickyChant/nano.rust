use std::time::Instant;

use nano_inference::{InferRequest, MockPredictor, Predictor, Tensor, TensorData};
use rayon::prelude::*;

fn main() {
    let rows = 20_000usize;
    let cols = 8usize;
    let values = (0..rows * cols)
        .map(|i| ((i * 37 + 11) % 10_000) as f32 / 100.0)
        .collect::<Vec<_>>();
    let predictor = MockPredictor::new("bench");

    let serial_start = Instant::now();
    let serial = values
        .chunks(cols)
        .flat_map(|row| predict_one(&predictor, row))
        .collect::<Vec<_>>();
    let serial_wall = serial_start.elapsed();

    let parallel_start = Instant::now();
    let parallel = values
        .par_chunks(cols)
        .flat_map(|row| predict_one(&predictor, row))
        .collect::<Vec<_>>();
    let parallel_wall = parallel_start.elapsed();

    assert_eq!(serial, parallel);
    let speedup = serial_wall.as_secs_f64() / parallel_wall.as_secs_f64().max(f64::EPSILON);
    println!(
        "serial: {serial_wall:?}, parallel: {parallel_wall:?}, speedup: {speedup:.2}x, outputs identical"
    );
}

fn predict_one(predictor: &MockPredictor, row: &[f32]) -> Vec<f32> {
    let req = InferRequest {
        model: "bench".to_string(),
        inputs: vec![Tensor {
            name: "features".to_string(),
            shape: vec![1, row.len()],
            data: TensorData::F32(row.to_vec()),
        }],
    };
    let response = predictor.predict(&req).expect("mock prediction succeeds");
    match &response.outputs[0].data {
        TensorData::F32(values) => values.clone(),
        _ => unreachable!("mock predictor emits f32"),
    }
}
