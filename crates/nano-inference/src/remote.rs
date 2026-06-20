use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use crate::{
    DataType, InferError, InferRequest, InferResponse, ModelMeta, Predictor, Tensor, TensorData,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireApi {
    KServeV2,
    OpenAI,
}

#[derive(Debug, Clone)]
pub struct RemotePredictor {
    endpoint: Url,
    model: String,
    api: WireApi,
    agent: ureq::Agent,
}

impl RemotePredictor {
    pub fn new(endpoint: Url, model: impl Into<String>, api: WireApi) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(5))
            .timeout_read(Duration::from_secs(30))
            .timeout_write(Duration::from_secs(30))
            .build();
        Self {
            endpoint,
            model: model.into(),
            api,
            agent,
        }
    }

    pub fn endpoint(&self) -> &Url {
        &self.endpoint
    }

    fn predict_kserve(&self, req: &InferRequest) -> Result<InferResponse, InferError> {
        let model = self.request_model(req);
        let url = self
            .endpoint
            .join(&format!("v2/models/{model}/infer"))
            .map_err(|err| InferError::Transport(err.to_string()))?;
        let body = serde_json::to_string(&KServeRequest::from_infer(req)?)
            .map_err(|err| InferError::InvalidPayload(err.to_string()))?;
        let response = send_json(&self.agent, url.as_str(), &body)?;
        let parsed = serde_json::from_str::<KServeResponse>(&response)
            .map_err(|err| InferError::InvalidPayload(err.to_string()))?;
        parsed.into_infer(model)
    }

    fn predict_openai(&self, req: &InferRequest) -> Result<InferResponse, InferError> {
        let url = self
            .endpoint
            .join("v1/chat/completions")
            .map_err(|err| InferError::Transport(err.to_string()))?;
        let model = self.request_model(req);
        let prompt = serde_json::to_string(req)
            .map_err(|err| InferError::InvalidPayload(err.to_string()))?;
        let body = serde_json::json!({
            "model": model,
            "messages": [{"role": "user", "content": prompt}],
            "temperature": 0.0
        })
        .to_string();
        let mut request = self
            .agent
            .post(url.as_str())
            .set("Content-Type", "application/json");
        if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            request = request.set("Authorization", &format!("Bearer {key}"));
        }
        let response = match request.send_string(&body) {
            Ok(response) => response,
            Err(ureq::Error::Status(code, response)) => {
                return Err(InferError::ServerStatus {
                    code,
                    body: response.into_string().unwrap_or_default(),
                });
            }
            Err(err) => return Err(InferError::Transport(err.to_string())),
        };
        let response_body = response
            .into_string()
            .map_err(|err| InferError::Transport(err.to_string()))?;
        let parsed = serde_json::from_str::<OpenAiChatResponse>(&response_body)
            .map_err(|err| InferError::InvalidPayload(err.to_string()))?;
        let content_len = parsed
            .choices
            .first()
            .map(|choice| choice.message.content.len() as i64)
            .unwrap_or(0);
        Ok(InferResponse {
            model,
            outputs: vec![Tensor {
                name: "choices_text_len".to_string(),
                shape: vec![1],
                data: TensorData::I64(vec![content_len]),
            }],
        })
    }

    fn request_model(&self, req: &InferRequest) -> String {
        if req.model.is_empty() {
            self.model.clone()
        } else {
            req.model.clone()
        }
    }
}

impl Predictor for RemotePredictor {
    fn predict(&self, req: &InferRequest) -> Result<InferResponse, InferError> {
        match self.api {
            WireApi::KServeV2 => self.predict_kserve(req),
            WireApi::OpenAI => self.predict_openai(req),
        }
    }

    fn metadata(&self) -> ModelMeta {
        ModelMeta::default()
    }
}

fn send_json(agent: &ureq::Agent, url: &str, body: &str) -> Result<String, InferError> {
    match agent
        .post(url)
        .set("Content-Type", "application/json")
        .send_string(body)
    {
        Ok(response) => response
            .into_string()
            .map_err(|err| InferError::Transport(err.to_string())),
        Err(ureq::Error::Status(code, response)) => Err(InferError::ServerStatus {
            code,
            body: response.into_string().unwrap_or_default(),
        }),
        Err(err) => Err(InferError::Transport(err.to_string())),
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct KServeRequest {
    inputs: Vec<KServeTensor>,
}

impl KServeRequest {
    fn from_infer(req: &InferRequest) -> Result<Self, InferError> {
        Ok(Self {
            inputs: req
                .inputs
                .iter()
                .map(KServeTensor::from_tensor)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }

    pub(crate) fn into_infer(self, model: String) -> Result<InferRequest, InferError> {
        Ok(InferRequest {
            model,
            inputs: self
                .inputs
                .into_iter()
                .map(KServeTensor::into_tensor)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct KServeResponse {
    #[serde(default)]
    model_name: Option<String>,
    outputs: Vec<KServeTensor>,
}

impl KServeResponse {
    fn from_infer(response: &InferResponse) -> Result<Self, InferError> {
        Ok(Self {
            model_name: Some(response.model.clone()),
            outputs: response
                .outputs
                .iter()
                .map(KServeTensor::from_tensor)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }

    fn into_infer(self, fallback_model: String) -> Result<InferResponse, InferError> {
        Ok(InferResponse {
            model: self.model_name.unwrap_or(fallback_model),
            outputs: self
                .outputs
                .into_iter()
                .map(KServeTensor::into_tensor)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct KServeTensor {
    name: String,
    shape: Vec<usize>,
    datatype: String,
    data: Value,
}

impl KServeTensor {
    fn from_tensor(tensor: &Tensor) -> Result<Self, InferError> {
        let datatype = match tensor.data.dtype() {
            DataType::F32 => "FP32",
            DataType::F64 => "FP64",
            DataType::I64 => "INT64",
        }
        .to_string();
        let data = match &tensor.data {
            TensorData::F32(values) => serde_json::to_value(values),
            TensorData::F64(values) => serde_json::to_value(values),
            TensorData::I64(values) => serde_json::to_value(values),
        }
        .map_err(|err| InferError::InvalidPayload(err.to_string()))?;
        Ok(Self {
            name: tensor.name.clone(),
            shape: tensor.shape.clone(),
            datatype,
            data,
        })
    }

    fn into_tensor(self) -> Result<Tensor, InferError> {
        let data = match self.datatype.as_str() {
            "FP32" => TensorData::F32(
                serde_json::from_value(self.data)
                    .map_err(|err| InferError::InvalidPayload(err.to_string()))?,
            ),
            "FP64" => TensorData::F64(
                serde_json::from_value(self.data)
                    .map_err(|err| InferError::InvalidPayload(err.to_string()))?,
            ),
            "INT64" => TensorData::I64(
                serde_json::from_value(self.data)
                    .map_err(|err| InferError::InvalidPayload(err.to_string()))?,
            ),
            other => {
                return Err(InferError::InvalidPayload(format!(
                    "unsupported KServe datatype {other}"
                )));
            }
        };
        Ok(Tensor {
            name: self.name,
            shape: self.shape,
            data,
        })
    }
}

#[derive(Debug, Serialize)]
struct SerializableRequest<'a> {
    model: &'a str,
    inputs: &'a [Tensor],
}

impl Serialize for Tensor {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        KServeTensor::from_tensor(self)
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
}

impl Serialize for InferRequest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        SerializableRequest {
            model: &self.model,
            inputs: &self.inputs,
        }
        .serialize(serializer)
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiChatResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAiMessage {
    content: String,
}

pub(crate) fn parse_kserve_request(body: &str, model: String) -> Result<InferRequest, InferError> {
    serde_json::from_str::<KServeRequest>(body)
        .map_err(|err| InferError::InvalidPayload(err.to_string()))?
        .into_infer(model)
}

pub(crate) fn serialize_kserve_response(response: &InferResponse) -> Result<String, InferError> {
    serde_json::to_string(&KServeResponse::from_infer(response)?)
        .map_err(|err| InferError::InvalidPayload(err.to_string()))
}
