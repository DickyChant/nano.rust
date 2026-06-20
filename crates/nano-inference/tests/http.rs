#![cfg(feature = "http")]

use nano_inference::{
    InferError, InferRequest, LaunchRecipe, ManagedPredictor, Predictor, Tensor, TensorData,
    WireApi,
};
use std::net::TcpStream;

#[test]
fn managed_roundtrip() {
    let predictor = match ManagedPredictor::start(
        LaunchRecipe::builtin_mock_ephemeral(),
        "managed_model",
        WireApi::KServeV2,
    ) {
        Ok(predictor) => predictor,
        Err(InferError::Transport(message)) if message.contains("Operation not permitted") => {
            eprintln!("skipping managed_roundtrip: localhost sockets are blocked in this sandbox");
            return;
        }
        Err(err) => panic!("failed to start managed predictor: {err}"),
    };
    let port = predictor.managed_port();
    let req = InferRequest {
        model: "managed_model".to_string(),
        inputs: vec![Tensor {
            name: "features".to_string(),
            shape: vec![2, 2],
            data: TensorData::F32(vec![1.0, 2.0, 3.0, 4.0]),
        }],
    };

    let first = predictor.predict(&req).unwrap();
    let second = predictor.predict(&req).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.model, "managed_model");
    assert_eq!(first.outputs[0].name, "score");
    assert_eq!(first.outputs[0].shape, vec![2, 1]);

    drop(predictor);
    assert!(
        TcpStream::connect(("127.0.0.1", port)).is_err(),
        "managed built-in mock server should stop on drop"
    );
}
