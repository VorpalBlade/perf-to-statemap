//! Parser for tracepoint format file under `/sys/kernel/tracing/events`.

use compact_str::CompactString;
use smallvec::SmallVec;
use std::borrow::Cow;

/// Error type for tracepoint format parsing.
#[derive(Debug, thiserror::Error)]
pub enum TracepointFormatError {
    /// Parse error in tracepoint format.
    #[error("Failed to parse tracepoint format: {0}")]
    ParseError(String),
    /// IO error when reading tracepoint format file.
    #[error("Failed to read tracepoint format file: {0}")]
    IoError(#[from] std::io::Error),
}

/// Represents a tracepoint format.
#[derive(Debug, Clone)]
pub struct TracepointFormat {
    /// The name of the tracepoint, e.g. `sched_switch`.
    #[allow(dead_code)]
    pub name: CompactString,
    /// The ID of the tracepoint.
    #[allow(dead_code)]
    pub id: u32,
    /// The print format string for the tracepoint.
    #[allow(dead_code)]
    pub print_fmt: String,
    /// The fields in the tracepoint format.
    pub fields: Vec<TracepointField>,
}

impl TracepointFormat {
    pub fn parse(lines: &str) -> Result<Self, TracepointFormatError> {
        #[derive(PartialEq, Eq)]
        enum Mode {
            Normal,
            Format,
        }

        let mut mode = Mode::Normal;
        let mut fields = Vec::new();
        let mut tp_name = CompactString::default();
        let mut id = 0;
        let mut print_fmt = String::new();

        for line in lines.split('\n') {
            if line.is_empty() {
                continue; // Skip empty lines
            }
            if mode == Mode::Format {
                if line.as_bytes()[0] == b'\t' {
                    let field = TracepointField::parse(line)?;
                    if let Some(field) = field {
                        fields.push(field);
                    }
                    continue;
                }
                mode = Mode::Normal; // End of format section
            }

            let (name, value) = line.split_once(":").ok_or_else(|| {
                TracepointFormatError::ParseError(
                    "Tracepoint format line does not contain a colon".to_string(),
                )
            })?;
            let value = value.strip_prefix(" ").unwrap_or(value); // Remove leading space
            match name {
                "name" => {
                    tp_name.push_str(value);
                }
                "ID" => {
                    id = value.parse::<u32>().map_err(|_| {
                        TracepointFormatError::ParseError("Invalid ID value".to_string())
                    })?;
                }
                "format" => {
                    mode = Mode::Format; // Switch to format mode
                    continue;
                }
                "print fmt" => {
                    // Store the print format
                    print_fmt.push_str(value);
                }
                _ => {
                    return Err(TracepointFormatError::ParseError(format!(
                        "Unknown tracepoint key: {name}"
                    )));
                }
            }
        }
        Ok(Self {
            name: tp_name,
            id,
            print_fmt,
            fields,
        })
    }
}

/// Represents the type of an array in a tracepoint format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TracepointArrayType {
    /// Not an array. Example: `char val; size:1;`
    None,
    /// Fixed (see `size`). Example: `char val[16]; size:16;`
    Fixed,
    /// Example: `char val[]; size:0;`
    /// The rest of the event is the array
    Trailing,
    /// Example: `__data_loc char[] val; size:4;`
    /// The upper byte is length, the lower byte is offset from start of
    /// tracepoint.
    DataLoc4,
    // Supposedly there is rel_loc (relative offset) and 2-byte versions of (rel/data) where the
    // length is strlen. I have not yet observed these in practice.
}

/// Represents a field in a tracepoint format.
#[derive(Debug, Clone)]
pub struct TracepointField {
    /// The C type of the field, e.g. `int`, `char[16]`, `__data_loc char[]`.
    #[allow(dead_code)]
    pub field_type: CompactString,
    /// The name of the field, e.g. `prev_comm`, `next_pid`.
    pub field_name: CompactString,
    /// The offset in bytes from the start of the record
    pub offset: u32,
    /// The size of the field in bytes
    pub size: u32,
    /// Whether the field is signed (e.g. `int` vs `unsigned int`).
    pub signed: bool,
    /// The type of array this field is, if any.
    pub array_type: TracepointArrayType,
}

static FIXED_REGEX: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"\[[0-9]+\]$").expect("Failed to compile regex")
});

impl TracepointField {
    fn parse(line: &str) -> Result<Option<Self>, TracepointFormatError> {
        if line.is_empty() {
            return Ok(None);
        }
        let parts: SmallVec<[&str; 4]> = line[1..].split('\t').collect();
        if parts.len() != 4 {
            return Err(TracepointFormatError::ParseError(format!(
                "Invalid tracepoint field format (expected 4 fields, got {})",
                parts.len()
            )));
        }
        let mut field_type = None;
        let mut field_name = None;
        let mut offset = 0;
        let mut size = 0;
        let mut signed = false;
        for field in parts {
            let field = field.strip_suffix(";").unwrap_or(field);
            let (name, value) = field.split_once(":").ok_or_else(|| {
                TracepointFormatError::ParseError(
                    "Field definition does not contain a colon".to_string(),
                )
            })?;
            match name {
                "field" => {
                    let last_space = value.rfind(' ').ok_or_else(|| {
                        TracepointFormatError::ParseError(
                            "Field definition does not contain a space".to_string(),
                        )
                    })?;
                    let ftype = &value[0..last_space];
                    field_type = Some(Cow::Borrowed(ftype));
                    let fname = &value[last_space + 1..];
                    field_name = Some(fname);
                    // If the field name ends with [number] move that to the
                    // type
                    if let Some(idx) = fname.rfind('[') {
                        field_type = Some(Cow::Owned(format!("{}{}", ftype, &fname[idx..])));
                        field_name = Some(&fname[..idx]);
                    }
                }
                "offset" => {
                    offset = value.parse::<u32>().map_err(|_| {
                        TracepointFormatError::ParseError("Invalid offset value".to_string())
                    })?;
                }
                "size" => {
                    size = value.parse::<u32>().map_err(|_| {
                        TracepointFormatError::ParseError("Invalid size value".to_string())
                    })?;
                }
                "signed" => {
                    signed = match value {
                        "1" => true,
                        "0" => false,
                        _ => {
                            return Err(TracepointFormatError::ParseError(
                                "Invalid signed value".to_string(),
                            ));
                        }
                    };
                }
                _ => {
                    return Err(TracepointFormatError::ParseError(format!(
                        "Unknown field: {name}"
                    )));
                }
            }
        }

        let field_type: CompactString = field_type
            .ok_or_else(|| TracepointFormatError::ParseError("Missing field type".to_string()))?
            .into();

        let array_type = if field_type.starts_with("__data_loc") && size == 4 {
            TracepointArrayType::DataLoc4
        } else if field_type.ends_with("[]") && size == 0 {
            TracepointArrayType::Trailing
        } else if FIXED_REGEX.is_match(&field_type) {
            TracepointArrayType::Fixed
        } else {
            TracepointArrayType::None
        };

        Ok(Some(Self {
            field_type,
            field_name: field_name
                .ok_or_else(|| TracepointFormatError::ParseError("Missing field name".to_string()))?
                .into(),
            offset,
            size,
            signed,
            array_type,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tracepoint_format_parse() {
        let input = indoc::indoc! {"
        name: sched_switch
        ID: 308
        format:
        \tfield:unsigned short common_type;\toffset:0;\tsize:2;\tsigned:0;
        \tfield:unsigned char common_flags;\toffset:2;\tsize:1;\tsigned:0;
        \tfield:unsigned char common_preempt_count;\toffset:3;\tsize:1;\tsigned:0;
        \tfield:int common_pid;\toffset:4;\tsize:4;\tsigned:1;

        \tfield:char prev_comm[16];\toffset:8;\tsize:16;\tsigned:0;
        \tfield:pid_t prev_pid;\toffset:24;\tsize:4;\tsigned:1;
        \tfield:int prev_prio;\toffset:28;\tsize:4;\tsigned:1;
        \tfield:long prev_state;\toffset:32;\tsize:8;\tsigned:1;
        \tfield:char next_comm[16];\toffset:40;\tsize:16;\tsigned:0;
        \tfield:pid_t next_pid;\toffset:56;\tsize:4;\tsigned:1;
        \tfield:int next_prio;\toffset:60;\tsize:4;\tsigned:1;

        print fmt: \"prev_comm=%s prev_pid=%d prev_prio=%d prev_state=%s%s ==> next_comm=%s next_pid=%d next_prio=%d\", REC->prev_comm, REC->prev_pid, REC->prev_prio, (REC->prev_state & ((((0x00000000 | 0x00000001 | 0x00000002 | 0x00000004 | 0x00000008 | 0x00000010 | 0x00000020 | 0x00000040) + 1) << 1) - 1)) ? __print_flags(REC->prev_state & ((((0x00000000 | 0x00000001 | 0x00000002 | 0x00000004 | 0x00000008 | 0x00000010 | 0x00000020 | 0x00000040) + 1) << 1) - 1), \"|\", { 0x00000001, \"S\" }, { 0x00000002, \"D\" }, { 0x00000004, \"T\" }, { 0x00000008, \"t\" }, { 0x00000010, \"X\" }, { 0x00000020, \"Z\" }, { 0x00000040, \"P\" }, { 0x00000080, \"I\" }) : \"R\", REC->prev_state & (((0x00000000 | 0x00000001 | 0x00000002 | 0x00000004 | 0x00000008 | 0x00000010 | 0x00000020 | 0x00000040) + 1) << 1) ? \"+\" : \"\", REC->next_comm, REC->next_pid, REC->next_prio
        "};
        let format = TracepointFormat::parse(input);
        insta::assert_debug_snapshot!(format);
    }

    #[test]
    fn test_tracepoint_field_parse() {
        let line = "\tfield:unsigned short common_type;\toffset:0;\tsize:2;\tsigned:0;";
        let field = TracepointField::parse(line);
        let field = field.unwrap().unwrap();
        assert_eq!(field.array_type, TracepointArrayType::None);
        assert_eq!(field.field_type, "unsigned short");
        assert_eq!(field.field_name, "common_type");
        assert_eq!(field.offset, 0);
        assert_eq!(field.size, 2);
        assert!(!field.signed);

        let line = "\tfield:int common_pid;\toffset:4;\tsize:4;\tsigned:1;";
        let field = TracepointField::parse(line).unwrap().unwrap();
        assert_eq!(field.array_type, TracepointArrayType::None);
        assert_eq!(field.field_type, "int");
        assert_eq!(field.field_name, "common_pid");
        assert_eq!(field.offset, 4);
        assert_eq!(field.size, 4);
        assert!(field.signed);

        let line = "\tfield:__data_loc char[] devname;\toffset:8;\tsize:4;\tsigned:0;";
        let field = TracepointField::parse(line).unwrap().unwrap();
        assert_eq!(field.array_type, TracepointArrayType::DataLoc4);
        assert_eq!(field.field_type, "__data_loc char[]");
        assert_eq!(field.field_name, "devname");
        assert_eq!(field.offset, 8);
        assert_eq!(field.size, 4);
        assert!(!field.signed);

        let line = "\tfield:char common_comm[16];\toffset:8;\tsize:16;\tsigned:0;";
        let field = TracepointField::parse(line).unwrap().unwrap();
        assert_eq!(field.array_type, TracepointArrayType::Fixed);
        assert_eq!(field.field_type, "char[16]");
        assert_eq!(field.field_name, "common_comm");
        assert_eq!(field.offset, 8);
        assert_eq!(field.size, 16);
        assert!(!field.signed);

        let line = "\tfield:char buf[];\toffset:16;\tsize:0;\tsigned:0;";
        let field = TracepointField::parse(line).unwrap().unwrap();
        assert_eq!(field.array_type, TracepointArrayType::Trailing);
        assert_eq!(field.field_type, "char[]");
        assert_eq!(field.field_name, "buf");
        assert_eq!(field.offset, 16);
        assert_eq!(field.size, 0);
        assert!(!field.signed);
    }
}
