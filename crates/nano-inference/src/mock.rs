use crate::tensor::{
    update_hash, DataType, InferError, InferRequest, InferResponse, ModelMeta, Predictor, Tensor,
    TensorData, TensorMeta, FNV_OFFSET,
};

#[derive(Debug, Clone)]
pub struct MockPredictor {
    model: String,
}

impl MockPredictor {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
        }
    }

    fn row_count(req: &InferRequest) -> Result<usize, InferError> {
        let Some(first) = req.inputs.first() else {
            return Ok(0);
        };
        let rows = first.shape.first().copied().unwrap_or(first.data.len());
        for tensor in &req.inputs {
            let actual: usize = tensor.shape.iter().product();
            if actual != tensor.data.len() {
                return Err(InferError::ShapeMismatch {
                    tensor: tensor.name.clone(),
                    expected: vec![tensor.data.len()],
                    actual: tensor.shape.clone(),
                });
            }
            if tensor.shape.first().copied().unwrap_or(tensor.data.len()) != rows {
                return Err(InferError::ShapeMismatch {
                    tensor: tensor.name.clone(),
                    expected: vec![rows],
                    actual: tensor.shape.clone(),
                });
            }
        }
        Ok(rows)
    }

    fn hash_row(req: &InferRequest, row: usize) -> u64 {
        let mut hash = FNV_OFFSET;
        update_hash(&mut hash, req.model.as_bytes());
        for tensor in &req.inputs {
            update_hash(&mut hash, tensor.name.as_bytes());
            update_hash(&mut hash, format!("{:?}", tensor.data.dtype()).as_bytes());
            for dim in tensor.shape.iter().skip(1) {
                update_hash(&mut hash, &dim.to_le_bytes());
            }
            let rows = tensor
                .shape
                .first()
                .copied()
                .unwrap_or(tensor.data.len())
                .max(1);
            let row_width = tensor.data.len() / rows;
            let start = row * row_width;
            let end = start + row_width;
            for value_index in start..end {
                tensor.data.hash_value_bytes(value_index, &mut hash);
            }
        }
        hash
    }
}

impl Default for MockPredictor {
    fn default() -> Self {
        Self::new("mock")
    }
}

impl Predictor for MockPredictor {
    fn predict(&self, req: &InferRequest) -> Result<InferResponse, InferError> {
        let rows = Self::row_count(req)?;
        let model = if req.model.is_empty() {
            self.model.clone()
        } else {
            req.model.clone()
        };
        let values = (0..rows)
            .map(|row| {
                let hash = Self::hash_row(req, row);
                (hash % 1_000_000) as f32 / 1_000_000.0
            })
            .collect::<Vec<_>>();
        Ok(InferResponse {
            model,
            outputs: vec![Tensor {
                name: "score".to_string(),
                shape: vec![rows, 1],
                data: TensorData::F32(values),
            }],
        })
    }

    fn metadata(&self) -> ModelMeta {
        ModelMeta {
            inputs: Vec::new(),
            outputs: vec![TensorMeta {
                name: "score".to_string(),
                dtype: DataType::F32,
                shape: vec![0, 1],
            }],
        }
    }
}
