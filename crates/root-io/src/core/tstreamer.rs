use std::fmt::Debug;

use nom::{
    combinator::{map, map_res},
    multi::{count, length_data, length_value},
    number::complete::*,
    IResult,
};

use crate::core::*;

/// Union of all posible `TStreamers`. See figure at
/// <https://root.cern.ch/doc/master/classTStreamerElement.html>
/// for inheritence of ROOT classes
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) enum TStreamer {
    Base {
        el: TStreamerElement,
        /// version number of the base class
        version_base: i32,
    },
    BasicType {
        el: TStreamerElement,
    },
    BasicPointer {
        el: TStreamerElement,
        /// version number of the class with the counter
        cvers: i32,
        /// name of data member holding the array count
        cname: String,
        /// name of the class with the counter
        ccls: String,
    },
    Loop {
        el: TStreamerElement,
        /// version number of the class with the counter
        cvers: i32,
        /// name of data member holding the array count
        cname: String,
        /// name of the class with the counter
        ccls: String,
    },
    Object {
        el: TStreamerElement,
    },
    ObjectPointer {
        el: TStreamerElement,
    },
    ObjectAny {
        el: TStreamerElement,
    },
    ObjectAnyPointer {
        el: TStreamerElement,
    },
    String {
        el: TStreamerElement,
    },
    Stl {
        el: TStreamerElement,
        /// type of STL vector
        vtype: StlTypeID,
        /// STL contained type
        ctype: TypeID,
    },
    StlString {
        el: TStreamerElement,
        /// type of STL vector
        vtype: StlTypeID,
        /// STL contained type
        ctype: TypeID,
    },
}

/// Every `TStreamer` inherits from `TStreamerElement`
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct TStreamerElement {
    ver: u16,
    name: TNamed,
    el_type: TypeID,
    size: i32,
    array_len: i32,
    array_dim: i32,
    max_idx: Vec<u32>,
    type_name: String,
    // For ver == 3
    // pub(crate) xmin: f32,
    // pub(crate) xmax: f32,
    // pub(crate) factor: f32,
}

/// Parse a `TStreamer` from a `Raw` buffer. This is usually the case
/// after reading the `TList` of `TStreamerInfo`s from a ROOT file
/// Parse a `TStreamer` from a `Raw` buffer. This is usually the case
/// after reading the `TList` of `TStreamerInfo`s from a ROOT file
pub(crate) fn tstreamer<'s>(raw: &Raw<'s>) -> IResult<&'s [u8], TStreamer> {
    let wrapped_tstreamerelem = parse_sized_object(tstreamerelement);
    let (i, _ver) = be_u16(raw.obj)?;
    match raw.classinfo {
        "TStreamerBase" => {
            let (i, el) = wrapped_tstreamerelem(i)?;
            let (i, version_base) = be_i32(i)?;
            Ok((i, TStreamer::Base { el, version_base }))
        }
        "TStreamerBasicType" => {
            let (i, el) = wrapped_tstreamerelem(i)?;
            Ok((i, TStreamer::BasicType { el }))
        }
        "TStreamerBasicPointer" => {
            let (i, el) = wrapped_tstreamerelem(i)?;
            let (i, cvers) = be_i32(i)?;
            let (i, cname) = string(i)?;
            let (i, ccls) = string(i)?;
            Ok((
                i,
                TStreamer::BasicPointer {
                    el,
                    cvers,
                    cname,
                    ccls,
                },
            ))
        }
        "TStreamerLoop" => {
            let (i, el) = wrapped_tstreamerelem(i)?;
            let (i, cvers) = be_i32(i)?;
            let (i, cname) = string(i)?;
            let (i, ccls) = string(i)?;
            Ok((
                i,
                TStreamer::Loop {
                    el,
                    cvers,
                    cname,
                    ccls,
                },
            ))
        }
        "TStreamerObject" => {
            let (i, el) = wrapped_tstreamerelem(i)?;
            Ok((i, TStreamer::Object { el }))
        }
        "TStreamerObjectPointer" => {
            let (i, el) = wrapped_tstreamerelem(i)?;
            Ok((i, TStreamer::ObjectPointer { el }))
        }
        "TStreamerObjectAny" => {
            let (i, el) = wrapped_tstreamerelem(i)?;
            Ok((i, TStreamer::ObjectAny { el }))
        }
        "TStreamerObjectAnyPointer" => {
            let (i, el) = wrapped_tstreamerelem(i)?;
            Ok((i, TStreamer::ObjectAnyPointer { el }))
        }
        "TStreamerString" => {
            let (i, el) = wrapped_tstreamerelem(i)?;
            Ok((i, TStreamer::String { el }))
        }
        "TStreamerSTL" => {
            let (i, el) = wrapped_tstreamerelem(i)?;
            let (i, vtype) = map(be_i32, StlTypeID::new)(i)?;
            let (i, ctype) = map_res(be_i32, |id| TypeID::new(id, &el.name.title))(i)?;
            Ok((i, TStreamer::Stl { el, vtype, ctype }))
        }
        "TStreamerSTLstring" => {
            // Two version bcs `stlstring` derives from `stl`
            let (i, _ver) = be_u16(raw.obj)?;
            let (_, stl_buffer) = length_data(checked_byte_count)(i)?;
            let (stl_buffer, _ver) = be_u16(stl_buffer)?;
            let (stl_buffer, el) = wrapped_tstreamerelem(stl_buffer)?;
            let (stl_buffer, vtype) = map(be_i32, StlTypeID::new)(stl_buffer)?;
            let (_stl_buffer, ctype) =
                map_res(be_i32, |id| TypeID::new(id, &el.name.title))(stl_buffer)?;
            Ok((i, TStreamer::StlString { el, vtype, ctype }))
        }
        ci => unimplemented!("Unknown TStreamer {}", ci),
    }
}

/// Return all `TSreamerInfo` for the data in this file
pub fn streamers<'s>(i: &'s [u8], ctx: &'s Context) -> IResult<&'s [u8], Vec<TStreamerInfo>> {
    // Dunno why we are 4 bytes off with the size of the streamer info...

    // This TList in the payload has a bytecount in front...
    let (i, tlist_objs) = length_value(checked_byte_count, |i| tlist(i, ctx))(i)?;
    // Mainly this is a TList of `TStreamerInfo`s, but there might
    // be some "rules" in the end
    let streamers = tlist_objs
        .iter()
        .filter_map(|raw| match raw.classinfo {
            "TStreamerInfo" => Some(raw.obj),
            _ => None,
        })
        .map(|i| tstreamerinfo(i, ctx).unwrap().1)
        .collect();
    // Parse the "rules", if any, from the same tlist
    let _rules: Vec<_> = tlist_objs
        .iter()
        .filter_map(|raw| match raw.classinfo {
            "TList" => Some(raw.obj),
            _ => None,
        })
        .map(|i| {
            let tl = tlist(i, ctx).unwrap().1;
            // Each `Rule` is a TList of `TObjString`s
            tl.iter()
                .map(|el| tobjstring(el.obj).unwrap().1)
                .collect::<Vec<_>>()
        })
        .collect();
    Ok((i, streamers))
}

/// The element which is wrapped in a TStreamer
fn tstreamerelement(i: &[u8]) -> IResult<&[u8], TStreamerElement> {
    let (i, ver) = be_u16(i)?;
    if ver <= 3 {
        unimplemented!();
    }
    let (i, name) = parse_sized_object(tnamed)(i)?;
    let (i, el_type) = map_res(be_i32, |id| TypeID::new(id, &name.title))(i)?;
    let (i, size) = be_i32(i)?;
    let (i, array_len) = be_i32(i)?;
    let (i, array_dim) = be_i32(i)?;
    let (i, max_idx) = match ver {
        1 => {
            let (i, n_times) = be_i32(i)?;
            count(be_u32, n_times as usize)(i)?
        }
        _ => count(be_u32, 5)(i)?,
    };
    let (i, type_name) = string(i)?;
    Ok((
        i,
        TStreamerElement {
            ver,
            name,
            el_type,
            size,
            array_len,
            array_dim,
            max_idx,
            type_name,
        },
    ))
}

pub(crate) fn type_is_core(name: &str) -> bool {
    match name {
        "TObject" | "TString" | "TNamed" | "TObjArray" | "TObjString" | "TList" => true,
        s => s.starts_with("TArray"),
    }
}

fn alias_or_lifetime(t: &str) -> String {
    if type_is_core(t) && t != "TObjArray" {
        return t.to_string();
    }
    format!("{}<'s>", t)
}

fn sanitize(n: &str) -> String {
    let keywords = [
        "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn",
        "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
        "return", "Self", "self", "static", "struct", "super", "trait", "true", "type", "unsafe",
        "use", "where", "while", "abstract", "alignof", "become", "box", "do", "final", "macro",
        "offsetof", "override", "priv", "proc", "pure", "sizeof", "typeof", "unsized", "virtual",
        "yield",
    ];
    if keywords.into_iter().any(|w| w == n) {
        format!("{n}_")
    } else {
        n.to_string()
    }
}

impl TStreamer {
    pub(crate) fn elem(&self) -> &TStreamerElement {
        use self::TStreamer::*;
        // TODO: Move element out of the enum
        match self {
            Base { ref el, .. }
            | BasicType { ref el }
            | BasicPointer { ref el, .. }
            | Loop { ref el, .. }
            | Object { ref el }
            | ObjectPointer { ref el }
            | ObjectAny { ref el }
            | ObjectAnyPointer { ref el }
            | String { ref el }
            | Stl { ref el, .. }
            | StlString { ref el, .. } => el,
        }
    }

    /// Get the comment associated with this particular member
    pub(crate) fn member_comment(&self) -> &str {
        &self.elem().name.title
    }

    /// The name of the member/field to be used in the generated struct
    pub(crate) fn member_name(&self) -> String {
        sanitize(&self.elem().name.name.to_lowercase())
    }

    pub(crate) fn type_name(&self) -> String {
        use self::TypeID::*;
        let name = alias_or_lifetime(&self.elem().name.name);
        match self {
            TStreamer::Base { ref el, .. } => {
                match el.el_type {
                    Object | Base | Named | TObject => name,
                    // Not sure about the following branch...
                    InvalidOrCounter(-1) => name,
                    _ => panic!("{:#?}", self),
                }
            }
            TStreamer::BasicType { ref el } => match el.el_type {
                Primitive(ref id) => id.type_name_str().to_string(),
                Offset(ref id) => format!("[{}; {}]", id.type_name_str(), el.array_len),
                _ => panic!("{:#?}", self),
            },
            TStreamer::BasicPointer { ref el, .. } => {
                match el.el_type {
                    Array(ref id) => {
                        // Arrays are preceeded by a byte and then have a length given by a
                        // previous member
                        format!("Vec<{}>", id.type_name_str())
                    }
                    _ => panic!("{:#?}", self),
                }
            }
            TStreamer::Object { ref el } => match el.el_type {
                Object => name,
                _ => panic!("{:#?}", self),
            },
            TStreamer::ObjectPointer { ref el } => {
                match el.el_type {
                    // Pointers may be null!
                    ObjectP | Objectp => format!("Option<{name}>"),
                    _ => panic!("{:#?}", self),
                }
            }
            TStreamer::ObjectAny { ref el } | &TStreamer::ObjectAnyPointer { ref el } => {
                match el.el_type {
                    Any | AnyP => name,
                    // No idea what this is; probably an array of custom type? Found in AliESDs
                    Unknown(82) => "Vec<u8>".to_string(),
                    _ => panic!("{:#?}", self),
                }
            }
            TStreamer::String { ref el } | TStreamer::StlString { ref el, .. } => {
                match el.el_type {
                    String | Streamer => "String".to_string(),
                    _ => panic!("{:#?}", self),
                }
            }
            TStreamer::Stl { ref vtype, .. } => match vtype {
                StlTypeID::Vector => "Stl_vec".to_string(),
                StlTypeID::Bitset => "Stl_bitset".to_string(),
                StlTypeID::String => "Stl_string".to_string(),
                StlTypeID::Map | StlTypeID::MultiMap => "Stl_map".to_string(),
            },
            _ => panic!("{:#?}", self),
        }
    }
}
