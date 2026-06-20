use std::error::Error as StdError;
use std::fmt;

/// The entire inference contract. Send tensors, get tensors.
pub trait Predictor: Send + Sync {
    fn predict(&self, req: &InferRequest) -> Result<InferResponse, InferError>;
    fn metadata(&self) -> ModelMeta;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    F32,
    F64,
    I64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tensor {
    pub name: String,
    pub shape: Vec<usize>,
    pub data: TensorData,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TensorData {
    F32(Vec<f32>),
    F64(Vec<f64>),
    I64(Vec<i64>),
}

impl TensorData {
    pub fn dtype(&self) -> DataType {
        match self {
            Self::F32(_) => DataType::F32,
            Self::F64(_) => DataType::F64,
            Self::I64(_) => DataType::I64,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Self::F32(values) => values.len(),
            Self::F64(values) => values.len(),
            Self::I64(values) => values.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) fn hash_value_bytes(&self, index: usize, hash: &mut u64) {
        match self {
            Self::F32(values) => update_hash(hash, &values[index].to_le_bytes()),
            Self::F64(values) => update_hash(hash, &values[index].to_le_bytes()),
            Self::I64(values) => update_hash(hash, &values[index].to_le_bytes()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct InferRequest {
    pub model: String,
    pub inputs: Vec<Tensor>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InferResponse {
    pub model: String,
    pub outputs: Vec<Tensor>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TensorMeta {
    pub name: String,
    pub dtype: DataType,
    pub shape: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModelMeta {
    pub inputs: Vec<TensorMeta>,
    pub outputs: Vec<TensorMeta>,
}

#[derive(Debug)]
pub enum InferError {
    ShapeMismatch {
        tensor: String,
        expected: Vec<usize>,
        actual: Vec<usize>,
    },
    MissingInput {
        name: String,
    },
    Unsupported(String),
    Transport(String),
    ServerStatus {
        code: u16,
        body: String,
    },
    Timeout(String),
    InvalidPayload(String),
    Feature(String),
}

impl fmt::Display for InferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ShapeMismatch {
                tensor,
                expected,
                actual,
            } => write!(
                f,
                "shape mismatch for tensor {tensor}: expected {expected:?}, got {actual:?}"
            ),
            Self::MissingInput { name } => write!(f, "missing input tensor: {name}"),
            Self::Unsupported(message) => write!(f, "unsupported inference provider: {message}"),
            Self::Transport(message) => write!(f, "transport error: {message}"),
            Self::ServerStatus { code, body } => {
                write!(f, "server returned status {code}: {body}")
            }
            Self::Timeout(message) => write!(f, "inference provider timed out: {message}"),
            Self::InvalidPayload(message) => write!(f, "invalid inference payload: {message}"),
            Self::Feature(message) => write!(f, "feature extraction failed: {message}"),
        }
    }
}

impl StdError for InferError {}

pub(crate) const FNV_OFFSET: u64 = 0xcbf29ce484222325;
pub(crate) const FNV_PRIME: u64 = 0x100000001b3;

pub(crate) fn update_hash(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}
