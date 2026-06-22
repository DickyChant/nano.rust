//! Inference protocol, providers, and feature extraction for nano.rust.
//!
//! This crate intentionally speaks a small tensor protocol instead of owning a
//! model runtime. Providers implement [`Predictor`]; callers can use the same
//! interface for deterministic CI mocks, remote KServe/OpenAI-compatible
//! endpoints, or managed local servers.

mod features;
mod mock;
mod provider;
mod tensor;

#[cfg(feature = "http")]
mod managed;
#[cfg(feature = "http")]
mod remote;

pub use features::{events_to_infer_request, FeatureScope};
#[cfg(feature = "http")]
pub use managed::{BuiltInMockServer, LaunchRecipe, ManagedPredictor};
pub use mock::{mock_scores, MockPredictor};
pub use provider::{InProcessPredictor, ProviderSpec};
#[cfg(feature = "http")]
pub use remote::{RemotePredictor, WireApi};
pub use tensor::{
    DataType, InferError, InferRequest, InferResponse, ModelMeta, Predictor, Tensor, TensorData,
    TensorMeta,
};
