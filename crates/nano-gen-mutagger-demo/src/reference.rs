use nano_analysis::Hist1D;
use nano_core::Event;
use nano_inference::{InferError, InferRequest, Predictor, Tensor, TensorData};

pub const MODEL_NAME: &str = "muon_tagger";
pub const MODEL_OUTPUT: &str = "topscore";

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReferenceRow {
    pub n_selected_muons: u32,
    pub n_tagged_muons: u32,
    pub leading_muon_pt: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceHistograms {
    pub leading_muon_pt: Hist1D,
}

impl ReferenceHistograms {
    pub fn new() -> Self {
        Self {
            leading_muon_pt: Hist1D::new(30, 30.0, 330.0),
        }
    }
}

impl Default for ReferenceHistograms {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ReferenceProducer;

impl ReferenceProducer {
    pub fn analyze(
        event: &Event,
        predictor: &impl Predictor,
    ) -> Result<Option<ReferenceRow>, InferError> {
        let pt = event.vector_ref::<f32>("Muon_pt").map_err(core_error)?;
        let eta = event.vector_ref::<f32>("Muon_eta").map_err(core_error)?;
        let phi = event.vector_ref::<f32>("Muon_phi").map_err(core_error)?;
        let scores = mock_tagger_scores(pt, eta, phi, predictor)?;

        let mut n_selected_muons = 0_u32;
        let mut n_tagged_muons = 0_u32;
        let mut leading_muon_pt = None::<f64>;

        for (((&pt, &eta), &score), _phi) in pt.iter().zip(eta).zip(&scores).zip(phi) {
            let pt = f64::from(pt);
            let eta = f64::from(eta);
            if pt > 30.0 && eta.abs() < 2.4 {
                n_selected_muons += 1;
                leading_muon_pt = Some(leading_muon_pt.map_or(pt, |lead| lead.max(pt)));
                if score > 0.5 {
                    n_tagged_muons += 1;
                }
            }
        }

        if n_tagged_muons < 1 {
            return Ok(None);
        }

        Ok(Some(ReferenceRow {
            n_selected_muons,
            n_tagged_muons,
            leading_muon_pt: leading_muon_pt.unwrap_or(0.0),
        }))
    }

    pub fn analyze_and_fill(
        event: &Event,
        predictor: &impl Predictor,
        histograms: &mut ReferenceHistograms,
    ) -> Result<Option<ReferenceRow>, InferError> {
        let row = Self::analyze(event, predictor)?;
        if let Some(row) = row {
            histograms
                .leading_muon_pt
                .fill_weighted(row.leading_muon_pt, 1.0);
        }
        Ok(row)
    }
}

fn mock_tagger_scores(
    pt: &[f32],
    eta: &[f32],
    phi: &[f32],
    predictor: &impl Predictor,
) -> Result<Vec<f32>, InferError> {
    let mut values = Vec::with_capacity(pt.len() * 3);
    for ((&pt, &eta), &phi) in pt.iter().zip(eta).zip(phi) {
        values.push(pt);
        values.push(eta);
        values.push(phi);
    }
    let response = predictor.predict(&InferRequest {
        model: MODEL_NAME.to_string(),
        inputs: vec![Tensor {
            name: "features".to_string(),
            shape: vec![pt.len(), 3],
            data: TensorData::F32(values),
        }],
    })?;
    let output = response.outputs.first().ok_or_else(|| {
        InferError::InvalidPayload(format!("model `{MODEL_NAME}` returned no outputs"))
    })?;
    let values = match &output.data {
        TensorData::F32(values) => values.clone(),
        other => {
            return Err(InferError::InvalidPayload(format!(
                "model `{MODEL_NAME}` output `{}` has dtype {:?}, expected F32",
                output.name,
                other.dtype()
            )));
        }
    };
    if values.len() != pt.len() {
        return Err(InferError::ShapeMismatch {
            tensor: output.name.clone(),
            expected: vec![pt.len()],
            actual: output.shape.clone(),
        });
    }
    Ok(values)
}

fn core_error(error: impl std::fmt::Display) -> InferError {
    InferError::Feature(error.to_string())
}
