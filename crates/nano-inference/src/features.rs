use nano_core::{BranchType, Event, ObjectView};

use crate::{InferError, InferRequest, Tensor, TensorData};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeatureScope {
    Event,
    Object { collection: String },
}

impl FeatureScope {
    pub fn object(collection: impl Into<String>) -> Self {
        Self::Object {
            collection: collection.into(),
        }
    }
}

pub fn events_to_infer_request(
    model: impl Into<String>,
    events: &[Event],
    scope: FeatureScope,
    attributes: &[impl AsRef<str>],
) -> Result<InferRequest, InferError> {
    let attr_names = attributes.iter().map(AsRef::as_ref).collect::<Vec<_>>();
    let mut values = Vec::new();
    let mut rows = 0usize;

    match &scope {
        FeatureScope::Event => {
            values.reserve(events.len().saturating_mul(attr_names.len()));
            for event in events {
                for attr in &attr_names {
                    values.push(read_event_numeric(event, attr)?);
                }
                rows += 1;
            }
        }
        FeatureScope::Object { collection } => {
            for event in events {
                let objects = event
                    .collection(collection)
                    .map_err(|err| InferError::Feature(err.to_string()))?;
                values.reserve(objects.len().saturating_mul(attr_names.len()));
                for object in objects.iter() {
                    for attr in &attr_names {
                        values.push(read_object_numeric(event, collection, attr, object)?);
                    }
                    rows += 1;
                }
            }
        }
    }

    Ok(InferRequest {
        model: model.into(),
        inputs: vec![Tensor {
            name: "features".to_string(),
            shape: vec![rows, attr_names.len()],
            data: TensorData::F32(values),
        }],
    })
}

fn read_event_numeric(event: &Event, branch: &str) -> Result<f32, InferError> {
    let info = event
        .schema()
        .find(branch)
        .ok_or_else(|| InferError::MissingInput {
            name: branch.to_string(),
        })?;
    match info.branch_type {
        BranchType::I8 => event.scalar::<i8>(branch).map(f32::from),
        BranchType::U8 => event.scalar::<u8>(branch).map(f32::from),
        BranchType::I16 => event.scalar::<i16>(branch).map(f32::from),
        BranchType::U16 => event.scalar::<u16>(branch).map(f32::from),
        BranchType::I32 => event.scalar::<i32>(branch).map(|value| value as f32),
        BranchType::U32 => event.scalar::<u32>(branch).map(|value| value as f32),
        BranchType::I64 => event.scalar::<i64>(branch).map(|value| value as f32),
        BranchType::U64 => event.scalar::<u64>(branch).map(|value| value as f32),
        BranchType::F32 => event.scalar::<f32>(branch),
        other => {
            return Err(InferError::Feature(format!(
                "event branch {branch} has non-scalar numeric type {other:?}"
            )));
        }
    }
    .map_err(|err| InferError::Feature(err.to_string()))
}

fn read_object_numeric(
    event: &Event,
    collection: &str,
    attr: &str,
    object: &ObjectView<'_>,
) -> Result<f32, InferError> {
    let branch = format!("{collection}_{attr}");
    let info = event
        .schema()
        .find(&branch)
        .ok_or(InferError::MissingInput { name: branch })?;

    match info.branch_type {
        BranchType::VecI8 => object.get::<i8>(attr).map(f32::from),
        BranchType::VecU8 => object.get::<u8>(attr).map(f32::from),
        BranchType::VecI16 => object.get::<i16>(attr).map(f32::from),
        BranchType::VecU16 => object.get::<u16>(attr).map(f32::from),
        BranchType::VecI32 => object.get::<i32>(attr).map(|value| value as f32),
        BranchType::VecU32 => object.get::<u32>(attr).map(|value| value as f32),
        BranchType::VecI64 => object.get::<i64>(attr).map(|value| value as f32),
        BranchType::VecU64 => object.get::<u64>(attr).map(|value| value as f32),
        BranchType::VecF32 => object.get::<f32>(attr),
        other => {
            return Err(InferError::Feature(format!(
                "object branch {collection}_{attr} has non-vector numeric type {other:?}"
            )));
        }
    }
    .map_err(|err| InferError::Feature(err.to_string()))
}
