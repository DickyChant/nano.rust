//! Native Rust correctionlib evaluation for nano.rust.
//!
//! This crate implements the first typed-corrections slice: load correctionlib
//! v2 JSON payloads, evaluate a small native subset, and expose a typed wrapper
//! pattern for analysis code.

use flate2::read::GzDecoder;
use serde::Deserialize;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::error::Error as StdError;
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::Path;

/// Convenient result alias for correction evaluation.
pub type Result<T> = std::result::Result<T, CorrectionError>;

/// Typed correctionlib input value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Real(f64),
    Str(String),
    Int(i64),
}

impl From<f64> for Value {
    fn from(value: f64) -> Self {
        Self::Real(value)
    }
}

impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Self::Str(value.to_string())
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Self::Str(value)
    }
}

/// Errors from parsing and evaluating correctionlib payloads.
#[derive(Debug)]
pub enum CorrectionError {
    Io(std::io::Error),
    Json(serde_json::Error),
    MalformedJson(String),
    MissingCorrection(String),
    InputCount {
        correction: String,
        expected: usize,
        actual: usize,
    },
    TypeMismatch {
        input: String,
        expected: InputType,
        actual: &'static str,
    },
    UnknownInput(String),
    BinningEdges {
        input: String,
        edges: usize,
        content: usize,
    },
    BinningFlow {
        input: String,
        value: f64,
    },
    CategoryNoMatch {
        input: String,
        key: String,
    },
    Unsupported(String),
    Formula(String),
}

impl fmt::Display for CorrectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "I/O error: {error}"),
            Self::Json(error) => write!(f, "JSON error: {error}"),
            Self::MalformedJson(message) => write!(f, "malformed correctionlib JSON: {message}"),
            Self::MissingCorrection(name) => write!(f, "correction `{name}` was not found"),
            Self::InputCount {
                correction,
                expected,
                actual,
            } => write!(
                f,
                "correction `{correction}` expected {expected} inputs, got {actual}"
            ),
            Self::TypeMismatch {
                input,
                expected,
                actual,
            } => write!(
                f,
                "input `{input}` expected correctionlib type {expected:?}, got {actual}"
            ),
            Self::UnknownInput(input) => write!(f, "content references unknown input `{input}`"),
            Self::BinningEdges {
                input,
                edges,
                content,
            } => write!(
                f,
                "binning input `{input}` has {edges} edges for {content} content nodes"
            ),
            Self::BinningFlow { input, value } => {
                write!(f, "value {value} is outside binning input `{input}`")
            }
            Self::CategoryNoMatch { input, key } => {
                write!(
                    f,
                    "category input `{input}` has no entry/default for key {key}"
                )
            }
            Self::Unsupported(message) => write!(f, "unsupported correctionlib feature: {message}"),
            Self::Formula(message) => write!(f, "formula error: {message}"),
        }
    }
}

impl StdError for CorrectionError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for CorrectionError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for CorrectionError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

/// A correctionlib v2 CorrectionSet.
#[derive(Debug, Clone, Deserialize)]
pub struct CorrectionSet {
    pub schema_version: Option<u32>,
    pub corrections: Vec<Correction>,
}

impl CorrectionSet {
    /// Parse a correction set from a JSON string.
    pub fn from_json_str(input: &str) -> Result<Self> {
        Ok(serde_json::from_str(input)?)
    }

    /// Read a plain `.json` or gzipped `.json.gz` correctionlib payload.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let file = File::open(path)?;
        let mut input = String::new();
        if path.extension().and_then(|extension| extension.to_str()) == Some("gz") {
            GzDecoder::new(file).read_to_string(&mut input)?;
        } else {
            std::io::BufReader::new(file).read_to_string(&mut input)?;
        }
        Self::from_json_str(&input)
    }

    /// Return a correction by name.
    pub fn correction(&self, name: &str) -> Result<&Correction> {
        self.corrections
            .iter()
            .find(|correction| correction.name == name)
            .ok_or_else(|| CorrectionError::MissingCorrection(name.to_string()))
    }
}

/// A correctionlib Correction.
#[derive(Debug, Clone, Deserialize)]
pub struct Correction {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: Option<u32>,
    pub inputs: Vec<Variable>,
    pub output: Variable,
    pub data: Content,
}

impl Correction {
    /// Evaluate this correction with values ordered according to `self.inputs`.
    ///
    /// Analysis code should normally prefer a typed wrapper such as
    /// [`MuonIdCorrection`]. This method is the schema-level primitive.
    pub fn evaluate(&self, values: &[Value]) -> Result<f64> {
        if values.len() != self.inputs.len() {
            return Err(CorrectionError::InputCount {
                correction: self.name.clone(),
                expected: self.inputs.len(),
                actual: values.len(),
            });
        }

        for (input, value) in self.inputs.iter().zip(values) {
            validate_value(input, value)?;
        }

        let context = EvalContext {
            inputs: &self.inputs,
            values,
        };
        self.data.evaluate(&context)
    }
}

/// A correctionlib input or output variable.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Variable {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: InputType,
    #[serde(default)]
    pub description: String,
}

/// Correctionlib scalar type names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputType {
    Real,
    String,
    Int,
}

/// Supported correctionlib content nodes.
#[derive(Debug, Clone)]
pub enum Content {
    Constant(f64),
    Binning(Binning),
    Category(Category),
    Formula(Formula),
    FormulaRef(FormulaRef),
}

impl Content {
    fn evaluate(&self, context: &EvalContext<'_>) -> Result<f64> {
        match self {
            Self::Constant(value) => Ok(*value),
            Self::Binning(node) => node.evaluate(context),
            Self::Category(node) => node.evaluate(context),
            Self::Formula(node) => node.evaluate(context),
            Self::FormulaRef(node) => Err(CorrectionError::Unsupported(format!(
                "FormulaRef index {} requires generic formula resolution",
                node.index
            ))),
        }
    }
}

impl<'de> Deserialize<'de> for Content {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = JsonValue::deserialize(deserializer)?;
        parse_content(&value).map_err(serde::de::Error::custom)
    }
}

/// A correctionlib binning node.
#[derive(Debug, Clone)]
pub struct Binning {
    pub input: String,
    pub edges: EdgeSpec,
    pub content: Vec<Content>,
    pub flow: Flow,
}

impl Binning {
    fn evaluate(&self, context: &EvalContext<'_>) -> Result<f64> {
        let value = context.real(&self.input)?;
        let edges = match &self.edges {
            EdgeSpec::Explicit(edges) => edges,
            EdgeSpec::Uniform { .. } => {
                return Err(CorrectionError::Unsupported(
                    "uniform binning edges are parsed but not evaluated in this slice".to_string(),
                ))
            }
        };

        if edges.len() != self.content.len() + 1 {
            return Err(CorrectionError::BinningEdges {
                input: self.input.clone(),
                edges: edges.len(),
                content: self.content.len(),
            });
        }

        let bin = if value < edges[0] {
            self.flow
                .bin_for_underflow(&self.input, value, self.content.len())?
        } else if value >= edges[edges.len() - 1] {
            self.flow
                .bin_for_overflow(&self.input, value, self.content.len())?
        } else {
            FlowBin::Index(
                edges
                    .windows(2)
                    .position(|window| value >= window[0] && value < window[1])
                    .ok_or_else(|| CorrectionError::BinningFlow {
                        input: self.input.clone(),
                        value,
                    })?,
            )
        };

        match bin {
            FlowBin::Index(index) => self.content[index].evaluate(context),
            FlowBin::Node(node) => node.evaluate(context),
            FlowBin::Constant(value) => Ok(value),
        }
    }
}

/// Binning edge representation.
#[derive(Debug, Clone)]
pub enum EdgeSpec {
    Explicit(Vec<f64>),
    Uniform { n: usize, low: f64, high: f64 },
}

/// Binning flow behavior.
#[derive(Debug, Clone)]
pub enum Flow {
    Error,
    Clamp,
    Constant(f64),
    Node(Box<Content>),
}

impl Flow {
    fn bin_for_underflow(
        &self,
        input: &str,
        value: f64,
        content_len: usize,
    ) -> Result<FlowBin<'_>> {
        match self {
            Self::Error => Err(CorrectionError::BinningFlow {
                input: input.to_string(),
                value,
            }),
            Self::Clamp => Ok(FlowBin::Index(0)),
            Self::Constant(flow_value) => Ok(FlowBin::Constant(*flow_value)),
            Self::Node(node) => Ok(FlowBin::Node(node)),
        }
        .and_then(|bin| validate_flow_bin(bin, input, content_len, value))
    }

    fn bin_for_overflow(&self, input: &str, value: f64, content_len: usize) -> Result<FlowBin<'_>> {
        match self {
            Self::Error => Err(CorrectionError::BinningFlow {
                input: input.to_string(),
                value,
            }),
            Self::Clamp => Ok(FlowBin::Index(content_len.saturating_sub(1))),
            Self::Constant(flow_value) => Ok(FlowBin::Constant(*flow_value)),
            Self::Node(node) => Ok(FlowBin::Node(node)),
        }
        .and_then(|bin| validate_flow_bin(bin, input, content_len, value))
    }
}

fn validate_flow_bin<'a>(
    bin: FlowBin<'a>,
    input: &str,
    content_len: usize,
    value: f64,
) -> Result<FlowBin<'a>> {
    match bin {
        FlowBin::Index(index) if index >= content_len => Err(CorrectionError::BinningFlow {
            input: input.to_string(),
            value,
        }),
        other => Ok(other),
    }
}

enum FlowBin<'a> {
    Index(usize),
    Node(&'a Content),
    Constant(f64),
}

/// A correctionlib category node.
#[derive(Debug, Clone)]
pub struct Category {
    pub input: String,
    pub content: Vec<CategoryItem>,
    pub default: Option<Box<Content>>,
}

impl Category {
    fn evaluate(&self, context: &EvalContext<'_>) -> Result<f64> {
        let key = CategoryKey::from_value(context.value(&self.input)?)?;
        if let Some(item) = self.content.iter().find(|item| item.key == key) {
            return item.value.evaluate(context);
        }

        match &self.default {
            Some(default) => default.evaluate(context),
            None => Err(CorrectionError::CategoryNoMatch {
                input: self.input.clone(),
                key: key.to_string(),
            }),
        }
    }
}

/// One category key/value entry.
#[derive(Debug, Clone)]
pub struct CategoryItem {
    pub key: CategoryKey,
    pub value: Content,
}

/// A category key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CategoryKey {
    Str(String),
    Int(i64),
}

impl CategoryKey {
    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::Str(value) => Ok(Self::Str(value.clone())),
            Value::Int(value) => Ok(Self::Int(*value)),
            Value::Real(_) => Err(CorrectionError::Unsupported(
                "real-valued category keys are not supported".to_string(),
            )),
        }
    }
}

impl fmt::Display for CategoryKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Str(value) => write!(f, "`{value}`"),
            Self::Int(value) => write!(f, "{value}"),
        }
    }
}

/// A correctionlib Formula node.
#[derive(Debug, Clone)]
pub struct Formula {
    pub expression: String,
    pub parser: String,
    pub variables: Vec<String>,
    pub parameters: Vec<f64>,
}

impl Formula {
    fn evaluate(&self, context: &EvalContext<'_>) -> Result<f64> {
        if self.parser != "TFormula" {
            return Err(CorrectionError::Unsupported(format!(
                "formula parser `{}`",
                self.parser
            )));
        }

        let variables = self
            .variables
            .iter()
            .map(|name| context.real(name))
            .collect::<Result<Vec<_>>>()?;
        evaluate_formula(
            &self.expression,
            &self.variables,
            &variables,
            &self.parameters,
        )
    }
}

/// A parsed but deliberately unsupported correctionlib FormulaRef node.
#[derive(Debug, Clone)]
pub struct FormulaRef {
    pub index: usize,
    pub parameters: Vec<f64>,
}

struct EvalContext<'a> {
    inputs: &'a [Variable],
    values: &'a [Value],
}

impl EvalContext<'_> {
    fn value(&self, name: &str) -> Result<&Value> {
        self.inputs
            .iter()
            .position(|input| input.name == name)
            .map(|index| &self.values[index])
            .ok_or_else(|| CorrectionError::UnknownInput(name.to_string()))
    }

    fn real(&self, name: &str) -> Result<f64> {
        match self.value(name)? {
            Value::Real(value) => Ok(*value),
            Value::Int(value) => Ok(*value as f64),
            Value::Str(_) => Err(CorrectionError::TypeMismatch {
                input: name.to_string(),
                expected: InputType::Real,
                actual: "string",
            }),
        }
    }
}

fn validate_value(input: &Variable, value: &Value) -> Result<()> {
    match (input.kind, value) {
        (InputType::Real, Value::Real(_))
        | (InputType::String, Value::Str(_))
        | (InputType::Int, Value::Int(_)) => Ok(()),
        (expected, actual) => Err(CorrectionError::TypeMismatch {
            input: input.name.clone(),
            expected,
            actual: value_type_name(actual),
        }),
    }
}

fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Real(_) => "real",
        Value::Str(_) => "string",
        Value::Int(_) => "int",
    }
}

fn parse_content(value: &JsonValue) -> Result<Content> {
    if let Some(number) = value.as_f64() {
        return Ok(Content::Constant(number));
    }

    let object = value.as_object().ok_or_else(|| {
        CorrectionError::MalformedJson("content must be a number or object".into())
    })?;
    let node_type = string_field(object, "nodetype")?;
    match node_type {
        "binning" => parse_binning(object).map(Content::Binning),
        "category" => parse_category(object).map(Content::Category),
        "formula" => parse_formula(object).map(Content::Formula),
        "formularef" => parse_formula_ref(object).map(Content::FormulaRef),
        other => Err(CorrectionError::Unsupported(format!(
            "content nodetype `{other}`"
        ))),
    }
}

fn parse_binning(object: &serde_json::Map<String, JsonValue>) -> Result<Binning> {
    let input = string_field(object, "input")?.to_string();
    let edges = parse_edges(required_field(object, "edges")?)?;
    let content = required_field(object, "content")?
        .as_array()
        .ok_or_else(|| CorrectionError::MalformedJson("binning content must be an array".into()))?
        .iter()
        .map(parse_content)
        .collect::<Result<Vec<_>>>()?;
    let flow = object
        .get("flow")
        .map(parse_flow)
        .transpose()?
        .unwrap_or(Flow::Error);

    Ok(Binning {
        input,
        edges,
        content,
        flow,
    })
}

fn parse_edges(value: &JsonValue) -> Result<EdgeSpec> {
    if let Some(edges) = value.as_array() {
        return edges
            .iter()
            .map(|edge| {
                edge.as_f64().ok_or_else(|| {
                    CorrectionError::MalformedJson("all explicit bin edges must be numbers".into())
                })
            })
            .collect::<Result<Vec<_>>>()
            .map(EdgeSpec::Explicit);
    }

    let object = value.as_object().ok_or_else(|| {
        CorrectionError::MalformedJson("binning edges must be an array or object".into())
    })?;
    let n = usize_field(object, "n")?;
    let low = f64_field(object, "low")?;
    let high = f64_field(object, "high")?;
    Ok(EdgeSpec::Uniform { n, low, high })
}

fn parse_flow(value: &JsonValue) -> Result<Flow> {
    match value {
        JsonValue::String(value) if value == "error" => Ok(Flow::Error),
        JsonValue::String(value) if value == "clamp" => Ok(Flow::Clamp),
        JsonValue::Number(value) => value
            .as_f64()
            .map(Flow::Constant)
            .ok_or_else(|| CorrectionError::MalformedJson("flow number is not finite".into())),
        JsonValue::Object(_) => parse_content(value).map(|content| Flow::Node(Box::new(content))),
        other => Err(CorrectionError::MalformedJson(format!(
            "unsupported flow value {other}"
        ))),
    }
}

fn parse_category(object: &serde_json::Map<String, JsonValue>) -> Result<Category> {
    let input = string_field(object, "input")?.to_string();
    let content = required_field(object, "content")?
        .as_array()
        .ok_or_else(|| CorrectionError::MalformedJson("category content must be an array".into()))?
        .iter()
        .map(parse_category_item)
        .collect::<Result<Vec<_>>>()?;
    let default = object
        .get("default")
        .map(parse_content)
        .transpose()?
        .map(Box::new);

    Ok(Category {
        input,
        content,
        default,
    })
}

fn parse_category_item(value: &JsonValue) -> Result<CategoryItem> {
    let object = value
        .as_object()
        .ok_or_else(|| CorrectionError::MalformedJson("category entry must be an object".into()))?;
    let key_value = required_field(object, "key")?;
    let key = match key_value {
        JsonValue::String(value) => CategoryKey::Str(value.clone()),
        JsonValue::Number(value) => CategoryKey::Int(value.as_i64().ok_or_else(|| {
            CorrectionError::MalformedJson("category integer key must fit i64".into())
        })?),
        _ => {
            return Err(CorrectionError::MalformedJson(
                "category keys must be strings or integers".into(),
            ))
        }
    };
    let value = parse_content(required_field(object, "value")?)?;
    Ok(CategoryItem { key, value })
}

fn parse_formula(object: &serde_json::Map<String, JsonValue>) -> Result<Formula> {
    let expression = string_field(object, "expression")?.to_string();
    let parser = string_field(object, "parser")?.to_string();
    let variables = string_array_field(object, "variables")?;
    let parameters = number_array_field(object, "parameters")?;

    Ok(Formula {
        expression,
        parser,
        variables,
        parameters,
    })
}

fn parse_formula_ref(object: &serde_json::Map<String, JsonValue>) -> Result<FormulaRef> {
    let index = usize_field(object, "index")?;
    let parameters = number_array_field(object, "parameters")?;
    Ok(FormulaRef { index, parameters })
}

fn required_field<'a>(
    object: &'a serde_json::Map<String, JsonValue>,
    name: &str,
) -> Result<&'a JsonValue> {
    object
        .get(name)
        .ok_or_else(|| CorrectionError::MalformedJson(format!("missing field `{name}`")))
}

fn string_field<'a>(object: &'a serde_json::Map<String, JsonValue>, name: &str) -> Result<&'a str> {
    required_field(object, name)?
        .as_str()
        .ok_or_else(|| CorrectionError::MalformedJson(format!("field `{name}` must be a string")))
}

fn f64_field(object: &serde_json::Map<String, JsonValue>, name: &str) -> Result<f64> {
    required_field(object, name)?
        .as_f64()
        .ok_or_else(|| CorrectionError::MalformedJson(format!("field `{name}` must be a number")))
}

fn usize_field(object: &serde_json::Map<String, JsonValue>, name: &str) -> Result<usize> {
    required_field(object, name)?
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| CorrectionError::MalformedJson(format!("field `{name}` must be usize")))
}

fn string_array_field(
    object: &serde_json::Map<String, JsonValue>,
    name: &str,
) -> Result<Vec<String>> {
    required_field(object, name)?
        .as_array()
        .ok_or_else(|| CorrectionError::MalformedJson(format!("field `{name}` must be an array")))?
        .iter()
        .map(|value| {
            value.as_str().map(str::to_string).ok_or_else(|| {
                CorrectionError::MalformedJson(format!("field `{name}` must contain strings"))
            })
        })
        .collect()
}

fn number_array_field(object: &serde_json::Map<String, JsonValue>, name: &str) -> Result<Vec<f64>> {
    required_field(object, name)?
        .as_array()
        .ok_or_else(|| CorrectionError::MalformedJson(format!("field `{name}` must be an array")))?
        .iter()
        .map(|value| {
            value.as_f64().ok_or_else(|| {
                CorrectionError::MalformedJson(format!("field `{name}` must contain numbers"))
            })
        })
        .collect()
}

/// Data-taking year for typed correction wrappers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Year {
    Run2016,
    Run2017,
    Run2018,
    Label(String),
    Number(i64),
}

impl Year {
    fn as_label(&self) -> String {
        match self {
            Self::Run2016 => "Run2016".to_string(),
            Self::Run2017 => "Run2017".to_string(),
            Self::Run2018 => "Run2018".to_string(),
            Self::Label(value) => value.clone(),
            Self::Number(value) => value.to_string(),
        }
    }

    fn as_number(&self) -> i64 {
        match self {
            Self::Run2016 => 2016,
            Self::Run2017 => 2017,
            Self::Run2018 => 2018,
            Self::Label(value) => value.parse().unwrap_or(0),
            Self::Number(value) => *value,
        }
    }
}

/// Nominal/up/down variation axis for typed correction wrappers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variation {
    Nominal,
    Up,
    Down,
}

impl Variation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Nominal => "nominal",
            Self::Up => "up",
            Self::Down => "down",
        }
    }
}

/// Example typed input for a muon ID scale factor correction.
#[derive(Debug, Clone)]
pub struct MuonIdInput {
    pub pt: f64,
    pub eta: f64,
    pub year: Year,
    pub variation: Variation,
}

/// Typed wrapper around a correctionlib muon ID scale factor correction.
#[derive(Debug, Clone)]
pub struct MuonIdCorrection {
    correction: Correction,
}

impl MuonIdCorrection {
    pub fn new(correction: Correction) -> Self {
        Self { correction }
    }

    pub fn correction(&self) -> &Correction {
        &self.correction
    }

    pub fn evaluate(&self, input: MuonIdInput) -> Result<f64> {
        let mut values = Vec::with_capacity(self.correction.inputs.len());
        for variable in &self.correction.inputs {
            values.push(muon_input_value(variable, &input)?);
        }
        self.correction.evaluate(&values)
    }
}

fn muon_input_value(variable: &Variable, input: &MuonIdInput) -> Result<Value> {
    match variable.name.as_str() {
        "pt" => Ok(Value::Real(input.pt)),
        "eta" => Ok(Value::Real(input.eta)),
        "abseta" | "abs_eta" | "abs(eta)" => Ok(Value::Real(input.eta.abs())),
        "variation" | "syst" | "systematic" | "sf" => {
            Ok(Value::Str(input.variation.as_str().to_string()))
        }
        "year" | "era" => match variable.kind {
            InputType::String => Ok(Value::Str(input.year.as_label())),
            InputType::Int => Ok(Value::Int(input.year.as_number())),
            InputType::Real => Ok(Value::Real(input.year.as_number() as f64)),
        },
        other => Err(CorrectionError::Unsupported(format!(
            "MuonIdCorrection does not know how to map input `{other}`"
        ))),
    }
}

fn evaluate_formula(
    expression: &str,
    variable_names: &[String],
    variables: &[f64],
    parameters: &[f64],
) -> Result<f64> {
    let normalized = expression.replace("TMath::", "");
    let tokens = Lexer::new(&normalized).tokens()?;
    let mut parser = FormulaParser {
        tokens,
        position: 0,
        variable_names: variable_names
            .iter()
            .enumerate()
            .map(|(index, name)| (name.as_str(), index))
            .collect(),
        variables,
        parameters,
    };
    let value = parser.expression()?;
    if parser.position != parser.tokens.len() {
        return Err(CorrectionError::Formula(format!(
            "unexpected token {:?}",
            parser.tokens[parser.position]
        )));
    }
    Ok(value)
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Pow,
    LParen,
    RParen,
    Comma,
    LBracket,
    RBracket,
}

struct Lexer<'a> {
    input: &'a str,
    index: usize,
}

impl<'a> Lexer<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, index: 0 }
    }

    fn tokens(mut self) -> Result<Vec<Token>> {
        let mut tokens = Vec::new();
        while let Some(character) = self.peek_char() {
            match character {
                ' ' | '\t' | '\n' | '\r' => {
                    self.advance_char();
                }
                '0'..='9' | '.' => tokens.push(Token::Number(self.number()?)),
                'A'..='Z' | 'a'..='z' | '_' => tokens.push(Token::Ident(self.ident())),
                '+' => {
                    self.advance_char();
                    tokens.push(Token::Plus);
                }
                '-' => {
                    self.advance_char();
                    tokens.push(Token::Minus);
                }
                '*' => {
                    self.advance_char();
                    if self.peek_char() == Some('*') {
                        self.advance_char();
                        tokens.push(Token::Pow);
                    } else {
                        tokens.push(Token::Star);
                    }
                }
                '/' => {
                    self.advance_char();
                    tokens.push(Token::Slash);
                }
                '^' => {
                    self.advance_char();
                    tokens.push(Token::Pow);
                }
                '(' => {
                    self.advance_char();
                    tokens.push(Token::LParen);
                }
                ')' => {
                    self.advance_char();
                    tokens.push(Token::RParen);
                }
                ',' => {
                    self.advance_char();
                    tokens.push(Token::Comma);
                }
                '[' => {
                    self.advance_char();
                    tokens.push(Token::LBracket);
                }
                ']' => {
                    self.advance_char();
                    tokens.push(Token::RBracket);
                }
                _ => {
                    return Err(CorrectionError::Formula(format!(
                        "unsupported character `{character}`"
                    )))
                }
            }
        }
        Ok(tokens)
    }

    fn peek_char(&self) -> Option<char> {
        self.input[self.index..].chars().next()
    }

    fn advance_char(&mut self) -> Option<char> {
        let character = self.peek_char()?;
        self.index += character.len_utf8();
        Some(character)
    }

    fn number(&mut self) -> Result<f64> {
        let start = self.index;
        while matches!(self.peek_char(), Some('0'..='9' | '.')) {
            self.advance_char();
        }
        if matches!(self.peek_char(), Some('e' | 'E')) {
            self.advance_char();
            if matches!(self.peek_char(), Some('+' | '-')) {
                self.advance_char();
            }
            while matches!(self.peek_char(), Some('0'..='9')) {
                self.advance_char();
            }
        }
        self.input[start..self.index].parse::<f64>().map_err(|_| {
            CorrectionError::Formula(format!(
                "invalid number `{}`",
                &self.input[start..self.index]
            ))
        })
    }

    fn ident(&mut self) -> String {
        let start = self.index;
        while matches!(
            self.peek_char(),
            Some('A'..='Z' | 'a'..='z' | '0'..='9' | '_')
        ) {
            self.advance_char();
        }
        self.input[start..self.index].to_string()
    }
}

struct FormulaParser<'a> {
    tokens: Vec<Token>,
    position: usize,
    variable_names: HashMap<&'a str, usize>,
    variables: &'a [f64],
    parameters: &'a [f64],
}

impl FormulaParser<'_> {
    fn expression(&mut self) -> Result<f64> {
        self.add_sub()
    }

    fn add_sub(&mut self) -> Result<f64> {
        let mut value = self.mul_div()?;
        loop {
            if self.consume(&Token::Plus) {
                value += self.mul_div()?;
            } else if self.consume(&Token::Minus) {
                value -= self.mul_div()?;
            } else {
                return Ok(value);
            }
        }
    }

    fn mul_div(&mut self) -> Result<f64> {
        let mut value = self.power()?;
        loop {
            if self.consume(&Token::Star) {
                value *= self.power()?;
            } else if self.consume(&Token::Slash) {
                value /= self.power()?;
            } else {
                return Ok(value);
            }
        }
    }

    fn power(&mut self) -> Result<f64> {
        let base = self.unary()?;
        if self.consume(&Token::Pow) {
            Ok(base.powf(self.power()?))
        } else {
            Ok(base)
        }
    }

    fn unary(&mut self) -> Result<f64> {
        if self.consume(&Token::Plus) {
            self.unary()
        } else if self.consume(&Token::Minus) {
            Ok(-self.unary()?)
        } else {
            self.primary()
        }
    }

    fn primary(&mut self) -> Result<f64> {
        match self.next() {
            Some(Token::Number(value)) => Ok(value),
            Some(Token::LParen) => {
                let value = self.expression()?;
                self.expect(Token::RParen)?;
                Ok(value)
            }
            Some(Token::LBracket) => {
                let index = self.parameter_index()?;
                self.expect(Token::RBracket)?;
                self.parameters.get(index).copied().ok_or_else(|| {
                    CorrectionError::Formula(format!("parameter index [{index}] is out of range"))
                })
            }
            Some(Token::Ident(name)) => self.identifier(&name),
            token => Err(CorrectionError::Formula(format!(
                "expected formula primary, got {token:?}"
            ))),
        }
    }

    fn identifier(&mut self, name: &str) -> Result<f64> {
        if self.consume(&Token::LParen) {
            return self.function(name);
        }

        if self.consume(&Token::LBracket) {
            let index = self.parameter_index()?;
            self.expect(Token::RBracket)?;
            return match name {
                "x" => self.indexed_variable(index),
                "param" | "p" => self.parameters.get(index).copied().ok_or_else(|| {
                    CorrectionError::Formula(format!("parameter index [{index}] is out of range"))
                }),
                other => Err(CorrectionError::Formula(format!(
                    "unsupported indexed identifier `{other}[{index}]`"
                ))),
            };
        }

        match name {
            "pi" | "Pi" | "PI" => Ok(std::f64::consts::PI),
            "e" => Ok(std::f64::consts::E),
            "x" => self.indexed_variable(0),
            "y" => self.indexed_variable(1),
            "z" => self.indexed_variable(2),
            "t" => self.indexed_variable(3),
            variable_name => self
                .variable_names
                .get(variable_name)
                .copied()
                .ok_or_else(|| {
                    CorrectionError::Formula(format!("unknown identifier `{variable_name}`"))
                })
                .and_then(|index| self.indexed_variable(index)),
        }
    }

    fn function(&mut self, name: &str) -> Result<f64> {
        let mut args = Vec::new();
        if !self.consume(&Token::RParen) {
            loop {
                args.push(self.expression()?);
                if self.consume(&Token::Comma) {
                    continue;
                }
                self.expect(Token::RParen)?;
                break;
            }
        }

        match (name, args.as_slice()) {
            ("sqrt", [x]) | ("Sqrt", [x]) => Ok(x.sqrt()),
            ("log", [x]) | ("Log", [x]) => Ok(x.ln()),
            ("log10", [x]) | ("Log10", [x]) => Ok(x.log10()),
            ("exp", [x]) | ("Exp", [x]) => Ok(x.exp()),
            ("abs", [x]) | ("Abs", [x]) => Ok(x.abs()),
            ("sin", [x]) | ("Sin", [x]) => Ok(x.sin()),
            ("cos", [x]) | ("Cos", [x]) => Ok(x.cos()),
            ("tan", [x]) | ("Tan", [x]) => Ok(x.tan()),
            ("pow", [x, y]) | ("Power", [x, y]) | ("Power_t", [x, y]) => Ok(x.powf(*y)),
            ("min", [x, y]) | ("Min", [x, y]) => Ok(x.min(*y)),
            ("max", [x, y]) | ("Max", [x, y]) => Ok(x.max(*y)),
            _ => Err(CorrectionError::Formula(format!(
                "unsupported function `{name}` with {} arguments",
                args.len()
            ))),
        }
    }

    fn indexed_variable(&self, index: usize) -> Result<f64> {
        self.variables.get(index).copied().ok_or_else(|| {
            CorrectionError::Formula(format!("variable index x[{index}] is out of range"))
        })
    }

    fn parameter_index(&mut self) -> Result<usize> {
        match self.next() {
            Some(Token::Number(value)) if value.fract() == 0.0 && value >= 0.0 => {
                Ok(value as usize)
            }
            token => Err(CorrectionError::Formula(format!(
                "expected non-negative integer index, got {token:?}"
            ))),
        }
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.position).cloned();
        if token.is_some() {
            self.position += 1;
        }
        token
    }

    fn consume(&mut self, token: &Token) -> bool {
        if self.tokens.get(self.position) == Some(token) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, token: Token) -> Result<()> {
        if self.consume(&token) {
            Ok(())
        } else {
            Err(CorrectionError::Formula(format!(
                "expected token {token:?}, got {:?}",
                self.tokens.get(self.position)
            )))
        }
    }
}
