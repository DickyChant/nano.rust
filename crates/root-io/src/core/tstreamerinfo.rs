use std::fmt::Debug;

use nom::{combinator::eof, multi::length_value, number::complete::*, IResult};

use crate::{core::tstreamer::type_is_core, core::*};

#[derive(Debug)]
#[allow(dead_code)]
pub struct TStreamerInfo {
    tstreamerinfo_ver: u16,
    named: TNamed,
    checksum: u32,
    new_class_version: u32,
    data_members: Vec<TStreamer>,
}

/// Parse one `TStreamerInfo` object (as found in the `TList`)
pub(crate) fn tstreamerinfo<'s>(
    i: &'s [u8],
    context: &'s Context,
) -> IResult<&'s [u8], TStreamerInfo> {
    let parse_members = |i| tobjarray(|raw_obj, _context| tstreamer(raw_obj), i, context);

    let (i, tstreamerinfo_ver) = be_u16(i)?;
    let (i, named) = length_value(checked_byte_count, tnamed)(i)?;
    let (i, checksum) = be_u32(i)?;
    let (i, new_class_version) = be_u32(i)?;
    let (i, _size_tobjarray_with_class_info) = checked_byte_count(i)?;
    let (i, _class_info_objarray) = classinfo(i)?;
    let (i, data_members) = length_value(checked_byte_count, parse_members)(i)?;
    let (i, _eof) = eof(i)?;
    Ok((
        i,
        TStreamerInfo {
            tstreamerinfo_ver,
            named,
            checksum,
            new_class_version,
            data_members,
        },
    ))
}

impl TStreamerInfo {
    pub(crate) fn to_yaml(&self) -> String {
        if type_is_core(self.named.name.as_str()) {
            return "".to_string();
        };
        let mut s = "".to_string();
        s += format!("{}:\n", self.named.name).as_str();
        s += format!("  version: {}\n", self.new_class_version).as_str();
        s += "  members:\n";
        for obj in &self.data_members {
            s += format!("      # {}\n", obj.member_comment()).as_str();
            s += format!("      {}: {}\n", obj.member_name(), obj.type_name()).as_str();
        }
        s += "\n";
        s
    }
}
