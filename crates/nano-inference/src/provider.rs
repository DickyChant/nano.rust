use std::path::PathBuf;

use crate::{InferError, InferRequest, InferResponse, MockPredictor, ModelMeta, Predictor};

#[derive(Debug, Clone)]
pub struct InProcessPredictor {
    onnx_path: PathBuf,
}

impl InProcessPredictor {
    pub fn new(onnx_path: impl Into<PathBuf>) -> Self {
        Self {
            onnx_path: onnx_path.into(),
        }
    }
}

impl Predictor for InProcessPredictor {
    fn predict(&self, _req: &InferRequest) -> Result<InferResponse, InferError> {
        Err(InferError::Unsupported(format!(
            "in-process ONNX inference for `{}` requires a future `onnx` feature",
            self.onnx_path.display()
        )))
    }

    fn metadata(&self) -> ModelMeta {
        ModelMeta::default()
    }
}

#[derive(Debug, Clone, Default)]
pub enum ProviderSpec {
    #[default]
    Mock,
    InProcess {
        onnx_path: PathBuf,
    },
    #[cfg(feature = "http")]
    Remote {
        endpoint: url::Url,
        api: crate::WireApi,
    },
    #[cfg(feature = "http")]
    Managed {
        launch: crate::LaunchRecipe,
        api: crate::WireApi,
    },
}

impl ProviderSpec {
    pub fn resolve(&self, model: impl Into<String>) -> Result<Box<dyn Predictor>, InferError> {
        let model = model.into();
        match self {
            Self::Mock => Ok(Box::new(MockPredictor::new(model))),
            Self::InProcess { onnx_path } => Ok(Box::new(InProcessPredictor::new(onnx_path))),
            #[cfg(feature = "http")]
            Self::Remote { endpoint, api } => Ok(Box::new(crate::RemotePredictor::new(
                endpoint.clone(),
                model,
                *api,
            ))),
            #[cfg(feature = "http")]
            Self::Managed { launch, api } => Ok(Box::new(crate::ManagedPredictor::start(
                launch.clone(),
                model,
                *api,
            )?)),
        }
    }
}
