use nano_producers::MuonSkimRow;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EntryRange {
    pub start: usize,
    pub end: usize,
}

impl EntryRange {
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChunkSpec {
    pub source: String,
    pub entry_range: EntryRange,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cutflow {
    pub events_seen: u64,
    pub events_selected: u64,
}

impl Cutflow {
    pub fn add_assign(&mut self, other: Self) {
        self.events_seen += other.events_seen;
        self.events_selected += other.events_selected;
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Histogram1D {
    pub name: String,
    pub bins: Vec<f64>,
    pub underflow: f64,
    pub overflow: f64,
}

impl Histogram1D {
    pub fn add_assign(&mut self, other: &Self) -> bool {
        if self.name != other.name || self.bins.len() != other.bins.len() {
            return false;
        }
        for (left, right) in self.bins.iter_mut().zip(&other.bins) {
            *left += right;
        }
        self.underflow += other.underflow;
        self.overflow += other.overflow;
        true
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PartialOutput {
    pub rows: Vec<MuonSkimRow>,
    pub cutflow: Cutflow,
    pub hists: Vec<Histogram1D>,
}

impl PartialOutput {
    pub fn merge(mut self, other: Self) -> Self {
        self.rows.extend(other.rows);
        self.cutflow.add_assign(other.cutflow);
        merge_histograms(&mut self.hists, other.hists);
        self
    }
}

impl Serialize for PartialOutput {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        PartialOutputCache::from(self.clone()).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PartialOutput {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(PartialOutputCache::deserialize(deserializer)?.into())
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MergedOutput {
    pub rows: Vec<MuonSkimRow>,
    pub cutflow: Cutflow,
    pub hists: Vec<Histogram1D>,
}

impl From<PartialOutput> for MergedOutput {
    fn from(value: PartialOutput) -> Self {
        Self {
            rows: value.rows,
            cutflow: value.cutflow,
            hists: value.hists,
        }
    }
}

impl Serialize for MergedOutput {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        MergedOutputCache::from(self.clone()).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for MergedOutput {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(MergedOutputCache::deserialize(deserializer)?.into())
    }
}

fn merge_histograms(left: &mut Vec<Histogram1D>, right: Vec<Histogram1D>) {
    for histogram in right {
        if let Some(existing) = left.iter_mut().find(|item| item.name == histogram.name) {
            assert!(
                existing.add_assign(&histogram),
                "histogram `{}` has incompatible bins",
                histogram.name
            );
        } else {
            left.push(histogram);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SerializableMuonSkimRow {
    n_good_muon: u32,
    lead_muon_pt: f32,
}

impl From<MuonSkimRow> for SerializableMuonSkimRow {
    fn from(value: MuonSkimRow) -> Self {
        Self {
            n_good_muon: value.n_good_muon,
            lead_muon_pt: value.lead_muon_pt,
        }
    }
}

impl From<SerializableMuonSkimRow> for MuonSkimRow {
    fn from(value: SerializableMuonSkimRow) -> Self {
        Self {
            n_good_muon: value.n_good_muon,
            lead_muon_pt: value.lead_muon_pt,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PartialOutputCache {
    rows: Vec<SerializableMuonSkimRow>,
    cutflow: Cutflow,
    hists: Vec<Histogram1D>,
}

impl From<PartialOutput> for PartialOutputCache {
    fn from(value: PartialOutput) -> Self {
        Self {
            rows: value.rows.into_iter().map(Into::into).collect(),
            cutflow: value.cutflow,
            hists: value.hists,
        }
    }
}

impl From<PartialOutputCache> for PartialOutput {
    fn from(value: PartialOutputCache) -> Self {
        Self {
            rows: value.rows.into_iter().map(Into::into).collect(),
            cutflow: value.cutflow,
            hists: value.hists,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MergedOutputCache {
    rows: Vec<SerializableMuonSkimRow>,
    cutflow: Cutflow,
    hists: Vec<Histogram1D>,
}

impl From<MergedOutput> for MergedOutputCache {
    fn from(value: MergedOutput) -> Self {
        Self {
            rows: value.rows.into_iter().map(Into::into).collect(),
            cutflow: value.cutflow,
            hists: value.hists,
        }
    }
}

impl From<MergedOutputCache> for MergedOutput {
    fn from(value: MergedOutputCache) -> Self {
        Self {
            rows: value.rows.into_iter().map(Into::into).collect(),
            cutflow: value.cutflow,
            hists: value.hists,
        }
    }
}
