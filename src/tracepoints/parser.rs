//! Code taking a [`super::format::TracepointFormat`] and making a parser
//! to a specific tracepoint struct.

use crate::tracepoints::format::TracepointArrayType;
use crate::tracepoints::format::TracepointField;
use crate::tracepoints::format::TracepointFormat;
use byteorder::ByteOrder;
use compact_str::CompactString;
use eyre::Context;
use linux_perf_data::linux_perf_event_reader::RawData;
use pastey::paste;
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;

/// Struct for applying parsing operations based on a tracepoint format.
#[derive(Debug, Clone)]
pub struct FormatParser {
    ops: Vec<ParseOp>,
}

/// Parser for a scalar type
macro_rules! scalar_parser {
    ($ty: tt) => {
        paste! {
            #[allow(dead_code)]
            pub fn [<parse_ $ty>]<O: ByteOrder>(
                &self,
                index: usize,
                record: &RawData<'_>,
            ) -> Result<$ty, std::io::Error> {
                let op = &self.ops[index];
                debug_assert!(
                    op.size as usize == size_of::<$ty>()
                        && op.array_type == TracepointArrayType::None
                        && op.signed,
                    "Tracepoint format mismatch"
                );
                let data = op.get_bytes(record)?;
                Ok(O::[<read_ $ty>](data.as_ref()))
            }
        }
    };
}

/// Parser for i8 and u8 types
macro_rules! byte_parser {
    ($ty: tt) => {
        paste! {
            #[allow(dead_code)]
            pub fn [<parse_ $ty>](
                &self,
                index: usize,
                record: &RawData<'_>,
            ) -> Result<$ty, std::io::Error> {
                let op = &self.ops[index];
                debug_assert!(
                    op.size as usize == size_of::<$ty>()
                        && op.array_type == TracepointArrayType::None
                        && op.signed,
                    "Tracepoint format mismatch"
                );
                let data = op.get_bytes(record)?;
                #[allow(trivial_numeric_casts)]
                Ok(data[0] as $ty)
            }
        }
    };
}

impl FormatParser {
    byte_parser!(i8);

    byte_parser!(u8);

    scalar_parser!(i16);

    scalar_parser!(u16);

    scalar_parser!(i32);

    scalar_parser!(u32);

    scalar_parser!(i64);

    scalar_parser!(u64);

    #[allow(dead_code)]
    pub fn parse_string<O: ByteOrder>(
        &self,
        index: usize,
        record: &RawData<'_>,
    ) -> Result<String, std::io::Error> {
        let data = self.parse_array::<O>(index, record)?;
        let data = data.as_ref();
        // Convert the C-style string to a Rust string
        let nulbyte = memchr::memchr(0, data).unwrap_or(data.len());
        let data = &data[..nulbyte];
        Ok(String::from_utf8_lossy(data).into_owned())
    }

    #[allow(non_snake_case)]
    pub fn parse_compact_string<O: ByteOrder>(
        &self,
        index: usize,
        record: &RawData<'_>,
    ) -> Result<CompactString, std::io::Error> {
        let data = self.parse_array::<O>(index, record)?;
        let data = data.as_ref();
        // Convert the C-style string to a Rust string
        let nulbyte = memchr::memchr(0, data).unwrap_or(data.len());
        let data = &data[..nulbyte];
        Ok(CompactString::from_utf8_lossy(data))
    }

    pub fn parse_array<'data, O: ByteOrder>(
        &self,
        index: usize,
        record: &RawData<'data>,
    ) -> Result<Cow<'data, [u8]>, std::io::Error> {
        let op = &self.ops[index];
        match op.array_type {
            TracepointArrayType::None => unreachable!("Expected an array type for a string field"),
            TracepointArrayType::Fixed => op.get_bytes(record),
            TracepointArrayType::Trailing => {
                op.get_bytes_range(record, record.len() - op.offset as usize)
            }
            TracepointArrayType::DataLoc4 => {
                let ptr = op.get_bytes(record)?;
                let ptr = O::read_u32(ptr.as_ref());
                let len = ptr >> 16;
                let ptr = ptr & 0xFFFF;
                Ok(record
                    .get(ptr as usize..(ptr + len) as usize)
                    .ok_or_else(|| {
                        std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "Not enough data")
                    })?
                    .as_slice())
            }
        }
    }

    /// Create a parser from the given tracepoint format file
    pub fn new(fields: &[TracepointField], names: &[&str]) -> Result<Self, eyre::Error> {
        let mapping: HashMap<&str, &TracepointField> = fields
            .iter()
            .map(|field| (field.field_name.as_str(), field))
            .collect();

        let ops = names
            .iter()
            .map(|name| {
                mapping
                    .get(name)
                    .ok_or_else(|| eyre::eyre!("Missing field: {}", name))
                    .map(|field| ParseOp::from(*field))
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self { ops })
    }
}

/// A parsing operation for a tracepoint field.
#[derive(Debug, Clone)]
struct ParseOp {
    offset: u32,
    size: u32,
    signed: bool,
    array_type: TracepointArrayType,
}

impl ParseOp {
    fn get_bytes<'data>(
        &self,
        record: &RawData<'data>,
    ) -> Result<Cow<'data, [u8]>, std::io::Error> {
        let data = record
            .get(self.offset as usize..(self.offset + self.size) as usize)
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "Not enough data")
            })?
            .as_slice();
        Ok(data)
    }

    fn get_bytes_range<'data>(
        &self,
        record: &RawData<'data>,
        length: usize,
    ) -> Result<Cow<'data, [u8]>, std::io::Error> {
        let data = record
            .get(self.offset as usize..self.offset as usize + length)
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "Not enough data")
            })?
            .as_slice();
        Ok(data)
    }
}

impl From<TracepointField> for ParseOp {
    fn from(field: TracepointField) -> Self {
        Self {
            offset: field.offset,
            size: field.size,
            signed: field.signed,
            array_type: field.array_type,
        }
    }
}

impl From<&TracepointField> for ParseOp {
    fn from(field: &TracepointField) -> Self {
        Self {
            offset: field.offset,
            size: field.size,
            signed: field.signed,
            array_type: field.array_type,
        }
    }
}

#[doc(hidden)]
#[allow(dead_code)]
pub fn make_parser_from_system(
    event_name: &str,
    parser_from_format: fn(&TracepointFormat) -> Result<FormatParser, eyre::Error>,
) -> Result<FormatParser, eyre::Error> {
    let (cat, name) = event_name
        .split_once(':')
        .ok_or_else(|| eyre::eyre!("Invalid event name: {}", event_name))?;
    let path = format!("/sys/kernel/tracing/events/{cat}/{name}/format");
    make_parser_from_file(Path::new(&path), parser_from_format)
}

#[doc(hidden)]
pub fn make_parser_from_sysroot(
    event_name: &str,
    sysroot_path: &Path,
    parser_from_format: fn(&TracepointFormat) -> Result<FormatParser, eyre::Error>,
) -> Result<FormatParser, eyre::Error> {
    let (cat, name) = event_name
        .split_once(':')
        .ok_or_else(|| eyre::eyre!("Invalid event name: {}", event_name))?;
    let path = format!("/sys/kernel/tracing/events/{cat}/{name}/format");
    make_parser_from_file(&sysroot_path.join(path), parser_from_format)
}

#[doc(hidden)]
pub fn make_parser_from_file(
    path: &Path,
    parser_from_format: fn(&TracepointFormat) -> Result<FormatParser, eyre::Error>,
) -> Result<FormatParser, eyre::Error> {
    let data = std::fs::read_to_string(path).wrap_err_with(|| {
        format!(
            "Failed to open \"{}\" (for loading tracepoint)",
            path.display()
        )
    })?;
    let format = TracepointFormat::parse(&data)?;
    parser_from_format(&format)
}

#[doc(hidden)]
#[macro_export]
macro_rules! struct_def {
    ($name:ident { $($field:ident: $type:ident,)* }) => {
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct $name {
            $(
                pub $field: $type,
            )*
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! common_members {
    ($event_name:tt, $name:ident { $($field:ident: $type:ident,)* }) => {
        const EVENT_NAME: &'static str = $event_name;

        fn parser_from_system() -> Result<$crate::tracepoints::parser::FormatParser, eyre::Error> {
            $crate::tracepoints::parser::make_parser_from_system(
                Self::EVENT_NAME,
                Self::parser_from_format,
            )
        }

        fn parser_from_file(path: &std::path::Path) -> Result<$crate::tracepoints::parser::FormatParser, eyre::Error> {
            $crate::tracepoints::parser::make_parser_from_file(path, Self::parser_from_format)
        }

        fn parser_from_sysroot<P: AsRef<std::path::Path>>(path: P) -> Result<$crate::tracepoints::parser::FormatParser, eyre::Error> {
            $crate::tracepoints::parser::make_parser_from_sysroot(
                Self::EVENT_NAME,
                path.as_ref(),
                Self::parser_from_format,
            )
        }

        fn parser_from_format(format: &$crate::tracepoints::format::TracepointFormat) -> Result<$crate::tracepoints::parser::FormatParser, eyre::Error> {
            static NAMES: &[&'static str] = &[
                $(
                    stringify!($field),
                )*
            ];
            $crate::tracepoints::parser::FormatParser::new(&format.fields, NAMES)
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! parser {
    ($format:ident $record:ident { $field:ident: $type:ident, $($tail:tt)* } @ $counter:tt @ $($result:tt)*) => {
        pastey::paste! {
            $crate::parser!(
                $format $record { $($tail)* }
                @ ($counter + 1)
                @ $($result)* $field: $format.[<parse_ $type:snake>]::<O>($counter, $record)?,);
        }
    };
    ($format:ident $record:ident { } @ $counter:tt @ $($result:tt)*) => {
        fn parse<O: byteorder::ByteOrder>(
            $format: &$crate::tracepoints::parser::FormatParser,
            $record: &linux_perf_data::linux_perf_event_reader::RawData<'_>,
        ) -> Result<Self, std::io::Error> {
            Ok(Self {
                $($result)*
            })
        }
    };
}

/// Macro to generate a parser for a tracepoint struct based on its format
/// loaded from disk.
#[doc(hidden)]
#[macro_export]
macro_rules! tracepoint_parser {
    (#[event_name($event_name:tt)] pub struct $name:ident { $($fields:tt)* }) => {
        $crate::struct_def!($name { $($fields)* });

        impl $crate::tracepoints::Tracepoint for $name {
            $crate::common_members!($event_name, $name { $($fields)* });
            $crate::parser!(format record { $($fields)* } @ 0 @ );
        }
    };
}

#[doc(inline)]
pub use tracepoint_parser;
