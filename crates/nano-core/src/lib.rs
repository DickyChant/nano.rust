//! nano-core — framework-level abstractions for the nano.rust event loop.
//!
//! This crate ports the C++ `include/nano/core` model into a small, idiomatic
//! Rust API:
//!
//! - [`BranchSchema`] records the explicitly declared input branches.
//! - [`Event`] owns one entry's branch-backed values plus dynamic attachments.
//! - [`Collection`] and [`ObjectView`] expose NanoAOD object attributes through
//!   the `Prefix_attr` grouping rule.

use std::any::Any;
use std::cell::{Ref, RefCell};
use std::collections::{HashMap, HashSet};
use std::error::Error as StdError;
use std::fmt;
use std::ops::Index;

/// Convenient result alias for the core event model.
pub type Result<T> = std::result::Result<T, NanoError>;

type AnyMap = HashMap<String, Box<dyn Any>>;
type ObjectExtraMap = HashMap<String, HashMap<usize, AnyMap>>;

/// Split a NanoAOD branch name into `(object, attribute)` per the grouping rule.
///
/// Vector branches named `Prefix_attr` map to object `Prefix` with attribute
/// `attr`. Names without an underscore (or non-collection branches such as
/// `Flag_goodVertices`, handled by the caller via the schema) are event-level
/// and return `None` here.
///
/// ```
/// use nano_core::split_branch_name;
/// assert_eq!(split_branch_name("FatJet_pt"), Some(("FatJet", "pt")));
/// assert_eq!(split_branch_name("Muon_miniPFRelIso_all"), Some(("Muon", "miniPFRelIso_all")));
/// assert_eq!(split_branch_name("MET"), None);
/// ```
pub fn split_branch_name(branch: &str) -> Option<(&str, &str)> {
    branch.split_once('_')
}

/// Declared NanoAOD branch value type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BranchType {
    Bool,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    VecBool,
    VecI8,
    VecU8,
    VecI16,
    VecU16,
    VecI32,
    VecU32,
    VecI64,
    VecU64,
    VecF32,
}

impl BranchType {
    /// Whether this branch type is a per-entry vector/jagged NanoAOD branch.
    pub fn is_vector(self) -> bool {
        matches!(
            self,
            Self::VecBool
                | Self::VecI8
                | Self::VecU8
                | Self::VecI16
                | Self::VecU16
                | Self::VecI32
                | Self::VecU32
                | Self::VecI64
                | Self::VecU64
                | Self::VecF32
        )
    }
}

/// One branch requested by a runtime card or producer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchSpec {
    pub name: String,
    pub branch_type: BranchType,
    pub optional: bool,
}

impl BranchSpec {
    pub fn new(name: impl Into<String>, branch_type: BranchType) -> Self {
        Self {
            name: name.into(),
            branch_type,
            optional: false,
        }
    }

    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }
}

/// Where a declared branch lands in the event model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchLocation {
    /// Scalar branches, and vector branches that do not match the object
    /// grouping rule, remain event-level data.
    Event,
    /// A vector branch `Object_attr` is exposed as `event.collection("Object")`
    /// and `obj.get::<T>("attr")`.
    Object {
        object_name: String,
        attribute_name: String,
    },
}

/// Resolved metadata for one declared branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchInfo {
    pub full_name: String,
    pub branch_type: BranchType,
    pub optional: bool,
    pub location: BranchLocation,
}

impl BranchInfo {
    pub fn is_event_level(&self) -> bool {
        matches!(self.location, BranchLocation::Event)
    }

    pub fn object_name(&self) -> Option<&str> {
        match &self.location {
            BranchLocation::Event => None,
            BranchLocation::Object { object_name, .. } => Some(object_name),
        }
    }

    pub fn attribute_name(&self) -> Option<&str> {
        match &self.location {
            BranchLocation::Event => None,
            BranchLocation::Object { attribute_name, .. } => Some(attribute_name),
        }
    }
}

/// Explicit branch declaration and object/attribute grouping.
#[derive(Debug, Clone)]
pub struct BranchSchema {
    specs: Vec<BranchSpec>,
    branches: HashMap<String, BranchInfo>,
    object_attributes: HashMap<String, Vec<String>>,
    aliases: HashMap<String, String>,
}

impl BranchSchema {
    /// Build a schema from the exact input branch list.
    pub fn new(specs: impl IntoIterator<Item = BranchSpec>) -> Result<Self> {
        let specs = specs.into_iter().collect::<Vec<_>>();
        let mut seen = HashSet::new();
        let mut branches = HashMap::new();
        let mut object_attributes: HashMap<String, Vec<String>> = HashMap::new();
        let mut aliases = HashMap::new();

        for spec in &specs {
            if !seen.insert(spec.name.clone()) {
                return Err(NanoError::DuplicateBranch {
                    branch: spec.name.clone(),
                });
            }

            let location = if spec.branch_type.is_vector() {
                match split_branch_name(&spec.name) {
                    Some((object_name, attribute_name))
                        if !object_name.is_empty() && !attribute_name.is_empty() =>
                    {
                        let object_name = object_name.to_string();
                        let attribute_name = attribute_name.to_string();
                        object_attributes
                            .entry(object_name.clone())
                            .or_default()
                            .push(attribute_name.clone());
                        aliases.insert(object_name.clone(), object_name.clone());
                        aliases.insert(Self::singularize(&object_name), object_name.clone());
                        aliases.insert(format!("{object_name}s"), object_name.clone());
                        BranchLocation::Object {
                            object_name,
                            attribute_name,
                        }
                    }
                    _ => BranchLocation::Event,
                }
            } else {
                BranchLocation::Event
            };

            branches.insert(
                spec.name.clone(),
                BranchInfo {
                    full_name: spec.name.clone(),
                    branch_type: spec.branch_type,
                    optional: spec.optional,
                    location,
                },
            );
        }

        Ok(Self {
            specs,
            branches,
            object_attributes,
            aliases,
        })
    }

    pub fn specs(&self) -> &[BranchSpec] {
        &self.specs
    }

    pub fn find(&self, full_name: impl AsRef<str>) -> Option<&BranchInfo> {
        self.branches.get(full_name.as_ref())
    }

    pub fn attributes_for_object(&self, object_name: impl AsRef<str>) -> Vec<&str> {
        let canonical = self.canonical_object_name(object_name.as_ref());
        self.object_attributes
            .get(canonical.as_str())
            .map(|attrs| attrs.iter().map(String::as_str).collect())
            .unwrap_or_default()
    }

    pub fn has_object(&self, object_name: impl AsRef<str>) -> bool {
        let canonical = self.canonical_object_name(object_name.as_ref());
        self.object_attributes.contains_key(canonical.as_str())
    }

    pub fn canonical_object_name(&self, requested: impl AsRef<str>) -> String {
        let requested = requested.as_ref();
        if let Some(canonical) = self.aliases.get(requested) {
            return canonical.clone();
        }
        let singular = Self::singularize(requested);
        self.aliases
            .get(&singular)
            .cloned()
            .unwrap_or_else(|| requested.to_string())
    }

    fn singularize(value: &str) -> String {
        value
            .strip_suffix('s')
            .map_or_else(|| value.to_string(), ToString::to_string)
    }
}

/// Owned in-memory column buffers, one row per event.
///
/// Vector variants represent NanoAOD jagged branches as `Vec<Vec<T>>`: outer
/// index is the event entry, inner index is the object within that entry.
#[derive(Debug, Clone, PartialEq)]
pub enum BranchColumn {
    Bool(Vec<bool>),
    I8(Vec<i8>),
    U8(Vec<u8>),
    I16(Vec<i16>),
    U16(Vec<u16>),
    I32(Vec<i32>),
    U32(Vec<u32>),
    I64(Vec<i64>),
    U64(Vec<u64>),
    F32(Vec<f32>),
    VecBool(Vec<Vec<bool>>),
    VecI8(Vec<Vec<i8>>),
    VecU8(Vec<Vec<u8>>),
    VecI16(Vec<Vec<i16>>),
    VecU16(Vec<Vec<u16>>),
    VecI32(Vec<Vec<i32>>),
    VecU32(Vec<Vec<u32>>),
    VecI64(Vec<Vec<i64>>),
    VecU64(Vec<Vec<u64>>),
    VecF32(Vec<Vec<f32>>),
}

impl BranchColumn {
    pub fn branch_type(&self) -> BranchType {
        match self {
            Self::Bool(_) => BranchType::Bool,
            Self::I8(_) => BranchType::I8,
            Self::U8(_) => BranchType::U8,
            Self::I16(_) => BranchType::I16,
            Self::U16(_) => BranchType::U16,
            Self::I32(_) => BranchType::I32,
            Self::U32(_) => BranchType::U32,
            Self::I64(_) => BranchType::I64,
            Self::U64(_) => BranchType::U64,
            Self::F32(_) => BranchType::F32,
            Self::VecBool(_) => BranchType::VecBool,
            Self::VecI8(_) => BranchType::VecI8,
            Self::VecU8(_) => BranchType::VecU8,
            Self::VecI16(_) => BranchType::VecI16,
            Self::VecU16(_) => BranchType::VecU16,
            Self::VecI32(_) => BranchType::VecI32,
            Self::VecU32(_) => BranchType::VecU32,
            Self::VecI64(_) => BranchType::VecI64,
            Self::VecU64(_) => BranchType::VecU64,
            Self::VecF32(_) => BranchType::VecF32,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Bool(v) => v.len(),
            Self::I8(v) => v.len(),
            Self::U8(v) => v.len(),
            Self::I16(v) => v.len(),
            Self::U16(v) => v.len(),
            Self::I32(v) => v.len(),
            Self::U32(v) => v.len(),
            Self::I64(v) => v.len(),
            Self::U64(v) => v.len(),
            Self::F32(v) => v.len(),
            Self::VecBool(v) => v.len(),
            Self::VecI8(v) => v.len(),
            Self::VecU8(v) => v.len(),
            Self::VecI16(v) => v.len(),
            Self::VecU16(v) => v.len(),
            Self::VecI32(v) => v.len(),
            Self::VecU32(v) => v.len(),
            Self::VecI64(v) => v.len(),
            Self::VecU64(v) => v.len(),
            Self::VecF32(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn vector_len_at(&self, entry: usize) -> Option<usize> {
        match self {
            Self::VecBool(v) => v.get(entry).map(Vec::len),
            Self::VecI8(v) => v.get(entry).map(Vec::len),
            Self::VecU8(v) => v.get(entry).map(Vec::len),
            Self::VecI16(v) => v.get(entry).map(Vec::len),
            Self::VecU16(v) => v.get(entry).map(Vec::len),
            Self::VecI32(v) => v.get(entry).map(Vec::len),
            Self::VecU32(v) => v.get(entry).map(Vec::len),
            Self::VecI64(v) => v.get(entry).map(Vec::len),
            Self::VecU64(v) => v.get(entry).map(Vec::len),
            Self::VecF32(v) => v.get(entry).map(Vec::len),
            _ => None,
        }
    }
}

/// Per-entry context, branch-backed values, and dynamic event/object extras.
pub struct Event {
    schema: BranchSchema,
    columns: HashMap<String, BranchColumn>,
    entry: usize,
    attachments: RefCell<AnyMap>,
    object_attachments: RefCell<ObjectExtraMap>,
}

impl Event {
    /// Construct one event view from in-memory column buffers.
    pub fn from_columns(
        schema: BranchSchema,
        columns: impl IntoIterator<Item = (impl Into<String>, BranchColumn)>,
        entry: usize,
    ) -> Result<Self> {
        let columns = columns
            .into_iter()
            .map(|(name, column)| (name.into(), column))
            .collect::<HashMap<_, _>>();

        for spec in schema.specs() {
            match columns.get(&spec.name) {
                Some(column) if column.branch_type() != spec.branch_type => {
                    return Err(NanoError::BranchTypeMismatch {
                        branch: spec.name.clone(),
                        expected: spec.branch_type,
                        actual: column.branch_type(),
                    });
                }
                Some(column) if entry >= column.len() => {
                    return Err(NanoError::EntryOutOfRange {
                        branch: spec.name.clone(),
                        entry,
                        len: column.len(),
                    });
                }
                Some(_) => {}
                None if spec.optional => {}
                None => {
                    return Err(NanoError::MissingBranch {
                        branch: spec.name.clone(),
                    });
                }
            }
        }

        Ok(Self {
            schema,
            columns,
            entry,
            attachments: RefCell::new(HashMap::new()),
            object_attachments: RefCell::new(HashMap::new()),
        })
    }

    pub fn entry(&self) -> usize {
        self.entry
    }

    pub fn schema(&self) -> &BranchSchema {
        &self.schema
    }

    pub fn has_physical_branch(&self, branch_name: impl AsRef<str>) -> bool {
        self.columns.contains_key(branch_name.as_ref())
    }

    pub fn is_mc(&self) -> bool {
        self.has_physical_branch("genWeight")
    }

    /// Read a scalar physical branch for the current entry.
    pub fn scalar<T: ScalarValue>(&self, branch_name: impl AsRef<str>) -> Result<T> {
        let branch_name = branch_name.as_ref();
        let column = self.column(branch_name)?;
        T::get_scalar(column, self.entry).ok_or_else(|| NanoError::TypeMismatch {
            name: branch_name.to_string(),
            expected: T::TYPE_NAME,
        })
    }

    /// Copy out a vector physical branch for the current entry.
    pub fn vector<T: ObjectValue>(&self, branch_name: impl AsRef<str>) -> Result<Vec<T>> {
        let branch_name = branch_name.as_ref();
        let column = self.column(branch_name)?;
        T::get_vector(column, self.entry).ok_or_else(|| NanoError::TypeMismatch {
            name: branch_name.to_string(),
            expected: T::VECTOR_TYPE_NAME,
        })
    }

    /// Borrow a numeric vector branch row for the current entry.
    ///
    /// `Vec<bool>` cannot be exposed as `&[bool]` because Rust stores
    /// `Vec<bool>` as packed bits; use [`Event::vector`] for that case.
    pub fn vector_ref<T: VectorSliceValue>(&self, branch_name: impl AsRef<str>) -> Result<&[T]> {
        let branch_name = branch_name.as_ref();
        let column = self.column(branch_name)?;
        T::get_vector_slice(column, self.entry).ok_or_else(|| NanoError::TypeMismatch {
            name: branch_name.to_string(),
            expected: T::VECTOR_TYPE_NAME,
        })
    }

    /// Access an object collection such as `FatJet`.
    pub fn collection(&self, object_name: impl AsRef<str>) -> Result<Collection<'_>> {
        let canonical = self.schema.canonical_object_name(object_name.as_ref());
        if !self.schema.has_object(&canonical) {
            return Err(NanoError::UnknownCollection { name: canonical });
        }

        let size = self
            .schema
            .attributes_for_object(&canonical)
            .into_iter()
            .filter_map(|attr| self.schema.find(format!("{canonical}_{attr}")))
            .filter_map(|info| self.columns.get(&info.full_name))
            .filter_map(|column| column.vector_len_at(self.entry))
            .max()
            .unwrap_or(0);

        Ok(Collection::new(self, canonical, size))
    }

    /// Attach derived event-level data.
    pub fn set<T: 'static>(&self, name: impl Into<String>, value: T) {
        self.attachments
            .borrow_mut()
            .insert(name.into(), Box::new(value));
    }

    pub fn has(&self, name: impl AsRef<str>) -> bool {
        self.attachments.borrow().contains_key(name.as_ref())
    }

    /// Read a typed event-level attachment.
    pub fn get<T: 'static>(&self, name: impl AsRef<str>) -> Result<Ref<'_, T>> {
        let name = name.as_ref().to_string();
        let attachments = self.attachments.borrow();
        if !attachments.contains_key(&name) {
            return Err(NanoError::MissingAttachment { name });
        }
        Ref::filter_map(attachments, |attachments| {
            attachments
                .get(&name)
                .and_then(|value| value.downcast_ref::<T>())
        })
        .map_err(|_| NanoError::TypeMismatch {
            name,
            expected: std::any::type_name::<T>(),
        })
    }

    fn column(&self, branch_name: &str) -> Result<&BranchColumn> {
        self.columns
            .get(branch_name)
            .ok_or_else(|| NanoError::MissingBranch {
                branch: branch_name.to_string(),
            })
    }
}

/// A light view over one NanoAOD object family.
pub struct Collection<'a> {
    object_name: String,
    objects: Vec<ObjectView<'a>>,
}

impl<'a> Collection<'a> {
    fn new(event: &'a Event, object_name: String, size: usize) -> Self {
        let objects = (0..size)
            .map(|index| ObjectView {
                event,
                object_name: object_name.clone(),
                index,
            })
            .collect();
        Self {
            object_name,
            objects,
        }
    }

    pub fn object_name(&self) -> &str {
        &self.object_name
    }

    pub fn len(&self) -> usize {
        self.objects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&ObjectView<'a>> {
        self.objects.get(index)
    }

    pub fn objects(&self) -> &[ObjectView<'a>] {
        &self.objects
    }

    pub fn iter(&self) -> impl Iterator<Item = &ObjectView<'a>> {
        self.objects.iter()
    }
}

impl<'a> Index<usize> for Collection<'a> {
    type Output = ObjectView<'a>;

    fn index(&self, index: usize) -> &Self::Output {
        &self.objects[index]
    }
}

impl<'a> IntoIterator for &'a Collection<'a> {
    type IntoIter = std::slice::Iter<'a, ObjectView<'a>>;
    type Item = &'a ObjectView<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.objects.iter()
    }
}

/// Indexed view of one object in a [`Collection`].
pub struct ObjectView<'a> {
    event: &'a Event,
    object_name: String,
    index: usize,
}

impl<'a> ObjectView<'a> {
    pub fn index(&self) -> usize {
        self.index
    }

    pub fn object_name(&self) -> &str {
        &self.object_name
    }

    /// Read an object attribute. Per-object dynamic attachments override raw
    /// branch-backed values when the requested type matches.
    pub fn get<T>(&self, attr: impl AsRef<str>) -> Result<T>
    where
        T: ObjectValue + Clone + 'static,
    {
        let attr = attr.as_ref();
        if let Some(value) = self.extra_cloned::<T>(attr)? {
            return Ok(value);
        }

        let branch_name = format!("{}_{}", self.object_name, attr);
        let column = self.event.column(&branch_name)?;
        T::get_object(column, self.event.entry, self.index).ok_or_else(|| NanoError::TypeMismatch {
            name: branch_name,
            expected: T::TYPE_NAME,
        })
    }

    pub fn pt(&self) -> Result<f32> {
        self.get("pt")
    }

    pub fn eta(&self) -> Result<f32> {
        self.get("eta")
    }

    pub fn phi(&self) -> Result<f32> {
        self.get("phi")
    }

    pub fn mass(&self) -> Result<f32> {
        self.get("mass")
    }

    /// Attach derived data to this object index.
    pub fn set<T: 'static>(&self, attr: impl Into<String>, value: T) {
        self.event
            .object_attachments
            .borrow_mut()
            .entry(self.object_name.clone())
            .or_default()
            .entry(self.index)
            .or_default()
            .insert(attr.into(), Box::new(value));
    }

    /// Read a typed per-object attachment.
    pub fn extra<T: 'static>(&self, attr: impl AsRef<str>) -> Result<Ref<'_, T>> {
        let attr = attr.as_ref().to_string();
        let object_name = self.object_name.clone();
        let index = self.index;
        let extras = self.event.object_attachments.borrow();
        let exists = extras
            .get(&object_name)
            .and_then(|by_index| by_index.get(&index))
            .is_some_and(|values| values.contains_key(&attr));
        if !exists {
            return Err(NanoError::MissingObjectAttachment {
                object: object_name,
                index,
                attr,
            });
        }

        Ref::filter_map(extras, |extras| {
            extras
                .get(&object_name)
                .and_then(|by_index| by_index.get(&index))
                .and_then(|values| values.get(&attr))
                .and_then(|value| value.downcast_ref::<T>())
        })
        .map_err(|_| NanoError::TypeMismatch {
            name: format!("{}[{}].{}", self.object_name, self.index, attr),
            expected: std::any::type_name::<T>(),
        })
    }

    fn extra_cloned<T: Clone + 'static>(&self, attr: &str) -> Result<Option<T>> {
        let extras = self.event.object_attachments.borrow();
        let Some(values) = extras
            .get(&self.object_name)
            .and_then(|by_index| by_index.get(&self.index))
        else {
            return Ok(None);
        };
        let Some(value) = values.get(attr) else {
            return Ok(None);
        };
        value
            .downcast_ref::<T>()
            .cloned()
            .map(Some)
            .ok_or_else(|| NanoError::TypeMismatch {
                name: format!("{}[{}].{}", self.object_name, self.index, attr),
                expected: std::any::type_name::<T>(),
            })
    }
}

/// Scalar values readable from event-level branch columns.
pub trait ScalarValue: Copy + 'static {
    const TYPE_NAME: &'static str;
    fn get_scalar(column: &BranchColumn, entry: usize) -> Option<Self>;
}

/// Element values readable from object/vector branch columns.
pub trait ObjectValue: Copy + 'static {
    const TYPE_NAME: &'static str;
    const VECTOR_TYPE_NAME: &'static str;
    fn get_object(column: &BranchColumn, entry: usize, index: usize) -> Option<Self>;
    fn get_vector(column: &BranchColumn, entry: usize) -> Option<Vec<Self>>;
}

/// Element values whose vector rows can be borrowed as slices.
pub trait VectorSliceValue: Copy + 'static {
    const VECTOR_TYPE_NAME: &'static str;
    fn get_vector_slice(column: &BranchColumn, entry: usize) -> Option<&[Self]>;
}

macro_rules! impl_values {
    ($ty:ty, $type_name:literal, $scalar_variant:ident, $vector_variant:ident) => {
        impl ScalarValue for $ty {
            const TYPE_NAME: &'static str = $type_name;

            fn get_scalar(column: &BranchColumn, entry: usize) -> Option<Self> {
                match column {
                    BranchColumn::$scalar_variant(values) => values.get(entry).copied(),
                    _ => None,
                }
            }
        }

        impl ObjectValue for $ty {
            const TYPE_NAME: &'static str = $type_name;
            const VECTOR_TYPE_NAME: &'static str = concat!("Vec<", $type_name, ">");

            fn get_object(column: &BranchColumn, entry: usize, index: usize) -> Option<Self> {
                match column {
                    BranchColumn::$vector_variant(values) => {
                        values.get(entry).and_then(|row| row.get(index)).copied()
                    }
                    _ => None,
                }
            }

            fn get_vector(column: &BranchColumn, entry: usize) -> Option<Vec<Self>> {
                match column {
                    BranchColumn::$vector_variant(values) => values.get(entry).cloned(),
                    _ => None,
                }
            }
        }
    };
}

macro_rules! impl_vector_slice {
    ($ty:ty, $type_name:literal, $vector_variant:ident) => {
        impl VectorSliceValue for $ty {
            const VECTOR_TYPE_NAME: &'static str = concat!("Vec<", $type_name, ">");

            fn get_vector_slice(column: &BranchColumn, entry: usize) -> Option<&[Self]> {
                match column {
                    BranchColumn::$vector_variant(values) => values.get(entry).map(Vec::as_slice),
                    _ => None,
                }
            }
        }
    };
}

impl_values!(bool, "bool", Bool, VecBool);
impl_values!(i8, "i8", I8, VecI8);
impl_values!(u8, "u8", U8, VecU8);
impl_values!(i16, "i16", I16, VecI16);
impl_values!(u16, "u16", U16, VecU16);
impl_values!(i32, "i32", I32, VecI32);
impl_values!(u32, "u32", U32, VecU32);
impl_values!(i64, "i64", I64, VecI64);
impl_values!(u64, "u64", U64, VecU64);
impl_values!(f32, "f32", F32, VecF32);

impl_vector_slice!(i8, "i8", VecI8);
impl_vector_slice!(u8, "u8", VecU8);
impl_vector_slice!(i16, "i16", VecI16);
impl_vector_slice!(u16, "u16", VecU16);
impl_vector_slice!(i32, "i32", VecI32);
impl_vector_slice!(u32, "u32", VecU32);
impl_vector_slice!(i64, "i64", VecI64);
impl_vector_slice!(u64, "u64", VecU64);
impl_vector_slice!(f32, "f32", VecF32);

/// Errors returned by schema construction and typed event/object access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NanoError {
    DuplicateBranch {
        branch: String,
    },
    MissingBranch {
        branch: String,
    },
    BranchTypeMismatch {
        branch: String,
        expected: BranchType,
        actual: BranchType,
    },
    EntryOutOfRange {
        branch: String,
        entry: usize,
        len: usize,
    },
    UnknownCollection {
        name: String,
    },
    MissingAttachment {
        name: String,
    },
    MissingObjectAttachment {
        object: String,
        index: usize,
        attr: String,
    },
    TypeMismatch {
        name: String,
        expected: &'static str,
    },
}

impl fmt::Display for NanoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateBranch { branch } => write!(f, "duplicate branch declaration: {branch}"),
            Self::MissingBranch { branch } => write!(f, "missing branch: {branch}"),
            Self::BranchTypeMismatch {
                branch,
                expected,
                actual,
            } => write!(
                f,
                "branch {branch} has type {actual:?}, expected {expected:?}"
            ),
            Self::EntryOutOfRange { branch, entry, len } => write!(
                f,
                "entry {entry} is out of range for branch {branch} with {len} rows"
            ),
            Self::UnknownCollection { name } => write!(f, "unknown collection: {name}"),
            Self::MissingAttachment { name } => write!(f, "missing attachment: {name}"),
            Self::MissingObjectAttachment {
                object,
                index,
                attr,
            } => write!(f, "missing object attachment: {object}[{index}].{attr}"),
            Self::TypeMismatch { name, expected } => {
                write!(f, "value {name} is not readable as {expected}")
            }
        }
    }
}

impl StdError for NanoError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> BranchSchema {
        BranchSchema::new([
            BranchSpec::new("FatJet_pt", BranchType::VecF32),
            BranchSpec::new("FatJet_eta", BranchType::VecF32),
            BranchSpec::new("FatJet_phi", BranchType::VecF32),
            BranchSpec::new("FatJet_mass", BranchType::VecF32),
            BranchSpec::new("FatJet_jetId", BranchType::VecU8),
            BranchSpec::new("Flag_goodVertices", BranchType::Bool),
            BranchSpec::new("MET_pt", BranchType::F32),
            BranchSpec::new("genWeight", BranchType::F32).optional(),
        ])
        .unwrap()
    }

    fn columns() -> Vec<(String, BranchColumn)> {
        vec![
            (
                "FatJet_pt".to_string(),
                BranchColumn::VecF32(vec![vec![350.0, 240.0], vec![125.0]]),
            ),
            (
                "FatJet_eta".to_string(),
                BranchColumn::VecF32(vec![vec![0.5, -1.5], vec![0.1]]),
            ),
            (
                "FatJet_phi".to_string(),
                BranchColumn::VecF32(vec![vec![1.0, -2.5], vec![2.0]]),
            ),
            (
                "FatJet_mass".to_string(),
                BranchColumn::VecF32(vec![vec![80.0, 60.0], vec![50.0]]),
            ),
            (
                "FatJet_jetId".to_string(),
                BranchColumn::VecU8(vec![vec![6, 2], vec![4]]),
            ),
            (
                "Flag_goodVertices".to_string(),
                BranchColumn::Bool(vec![true, false]),
            ),
            ("MET_pt".to_string(), BranchColumn::F32(vec![90.0, 75.0])),
        ]
    }

    #[test]
    fn groups_object_branches() {
        assert_eq!(split_branch_name("FatJet_pt"), Some(("FatJet", "pt")));
        assert_eq!(
            split_branch_name("Muon_miniPFRelIso_all"),
            Some(("Muon", "miniPFRelIso_all"))
        );
        assert_eq!(split_branch_name("MET"), None);
    }

    #[test]
    fn schema_groups_vector_branches_and_leaves_scalars_event_level() {
        let schema = schema();

        let pt = schema.find("FatJet_pt").unwrap();
        assert_eq!(pt.object_name(), Some("FatJet"));
        assert_eq!(pt.attribute_name(), Some("pt"));
        assert!(!pt.is_event_level());

        let flag = schema.find("Flag_goodVertices").unwrap();
        assert!(flag.is_event_level());
        assert_eq!(flag.object_name(), None);

        assert_eq!(
            schema.attributes_for_object("FatJets"),
            vec!["pt", "eta", "phi", "mass", "jetId"]
        );
        assert_eq!(schema.canonical_object_name("FatJets"), "FatJet");
    }

    #[test]
    fn event_reads_scalar_and_event_level_attachments() {
        let event = Event::from_columns(schema(), columns(), 0).unwrap();

        assert_eq!(event.scalar::<bool>("Flag_goodVertices").unwrap(), true);
        assert_eq!(event.scalar::<f32>("MET_pt").unwrap(), 90.0);
        assert_eq!(
            event.vector_ref::<f32>("FatJet_pt").unwrap(),
            &[350.0, 240.0]
        );
        assert!(!event.is_mc());

        event.set("ht", 590.0_f32);
        event.set("selected_count", 2_u32);

        assert!(event.has("ht"));
        assert_eq!(*event.get::<f32>("ht").unwrap(), 590.0);
        assert_eq!(*event.get::<u32>("selected_count").unwrap(), 2);
    }

    #[test]
    fn collection_indexed_access_and_convenience_accessors() {
        let event = Event::from_columns(schema(), columns(), 0).unwrap();
        let fatjets = event.collection("FatJets").unwrap();

        assert_eq!(fatjets.object_name(), "FatJet");
        assert_eq!(fatjets.len(), 2);
        assert_eq!(fatjets[0].index(), 0);
        assert_eq!(fatjets[0].pt().unwrap(), 350.0);
        assert_eq!(fatjets[1].eta().unwrap(), -1.5);
        assert_eq!(fatjets[1].phi().unwrap(), -2.5);
        assert_eq!(fatjets[0].mass().unwrap(), 80.0);
        assert_eq!(fatjets[0].get::<u8>("jetId").unwrap(), 6);
    }

    #[test]
    fn object_attachments_override_branch_reads_and_extra_typed_readback() {
        let event = Event::from_columns(schema(), columns(), 0).unwrap();
        let fatjets = event.collection("FatJet").unwrap();
        let jet = &fatjets[0];

        jet.set("pt", 401.0_f32);
        jet.set("p4", (401.0_f32, 0.5_f32, 1.0_f32, 80.0_f32));
        jet.set("is_qualified", true);

        assert_eq!(jet.get::<f32>("pt").unwrap(), 401.0);
        assert_eq!(
            *jet.extra::<(f32, f32, f32, f32)>("p4").unwrap(),
            (401.0, 0.5, 1.0, 80.0)
        );
        assert_eq!(*jet.extra::<bool>("is_qualified").unwrap(), true);
    }

    #[test]
    fn typed_get_reports_wrong_type() {
        let event = Event::from_columns(schema(), columns(), 0).unwrap();
        event.set("ht", 590.0_f32);
        assert!(matches!(
            event.get::<i32>("ht").unwrap_err(),
            NanoError::TypeMismatch { .. }
        ));

        assert!(matches!(
            event.scalar::<i32>("MET_pt").unwrap_err(),
            NanoError::TypeMismatch { .. }
        ));

        let fatjets = event.collection("FatJet").unwrap();
        fatjets[0].set("dr_T", 0.4_f32);
        assert!(matches!(
            fatjets[0].extra::<bool>("dr_T").unwrap_err(),
            NanoError::TypeMismatch { .. }
        ));
    }
}
