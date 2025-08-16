//! Support for tracepoint parsing and handling.

use std::path::Path;

pub mod format;
pub mod irq;
pub mod parser;
pub mod sched;

/// Trait for tracepoint structs that can be parsed from a tracepoint format.
pub trait Tracepoint {
    /// Name of the tracepoint this struct corresponds to. E.g.
    /// "`sched:sched_switch`".
    const EVENT_NAME: &'static str;

    /// Create a parser from the current system's tracepoint format.
    ///
    /// This needs read access to
    /// `/sys/kernel/tracing/events/<category>/<name>/format`,
    /// which by default is root only.
    #[allow(dead_code)]
    fn parser_from_system() -> Result<parser::FormatParser, eyre::Error>;

    /// Create a parser from a different system's tracepoint format.
    ///
    /// This needs read access to
    /// `<sysroot>/sys/kernel/tracing/events/<category>/<name>/format`
    fn parser_from_sysroot<P: AsRef<Path>>(path: P) -> Result<parser::FormatParser, eyre::Error>;

    /// Create a parser from the given tracepoint format file.
    #[allow(dead_code)]
    fn parser_from_file(path: &Path) -> Result<parser::FormatParser, eyre::Error>;

    /// Create a parser matching this struct for the given dynamic tracepoint
    /// format.
    fn parser_from_format(
        format: &format::TracepointFormat,
    ) -> Result<parser::FormatParser, eyre::Error>;

    /// Parse raw data using this struct
    fn parse<O: byteorder::ByteOrder>(
        format: &parser::FormatParser,
        record: &linux_perf_data::linux_perf_event_reader::RawData<'_>,
    ) -> Result<Self, std::io::Error>
    where
        Self: Sized;
}
