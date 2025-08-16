//! Struct definitions taken from the statemap project
//!
//! These were copy-pasted from [statemap](https://github.com/oxidecomputer/statemap)
//! and then modified to work with serialisation instead of deserialisation.
//!
//! Also, `StatemapInputDatum` was made generic over an enum type.

use compact_str::CompactString;
use serde_derive::Deserialize;
use serde_derive::Serialize;
use std::collections::HashMap;

/*
 * The StatemapInput* types denote the structure of the concatenated JSON
 * in the input file.
 */
#[derive(Serialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct StatemapInputState {
    pub color: Option<CompactString>, // color for state, if any
    pub value: usize,                 // value for state
}

#[derive(Serialize, Debug, Default, Clone)]
#[serde(deny_unknown_fields)]
pub struct StatemapInputDatum<T: serde::Serialize + Default + Copy + Clone + std::fmt::Debug> {
    #[serde(serialize_with = "serialize_as_string")]
    pub time: u64, // time of this datum
    pub entity: CompactString,      // name of entity
    pub state: T,                   // state entity is in at time
    pub tag: Option<CompactString>, // tag for this state, if any
}

// I'm not sure why the format uses strings for this. I guess it is because
// JS has issues with large integers since it uses floats...?
fn serialize_as_string<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let s = format!("{value}");
    serializer.serialize_str(&s)
}

#[derive(Serialize, Debug)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct StatemapInputDescription {
    entity: String,      // name of entity
    description: String, // description of entity
}

#[derive(Serialize, Debug)]
#[allow(non_snake_case)]
#[serde(deny_unknown_fields)]
pub struct StatemapInputMetadata {
    pub start: Vec<u64>,
    pub title: CompactString,
    pub host: Option<CompactString>,
    pub entityKind: Option<CompactString>,
    pub states: HashMap<CompactString, StatemapInputState>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct StatemapInputEvent {
    time: String,           // time of this datum
    entity: String,         // name of entity
    event: String,          // type of event
    target: Option<String>, // target for event, if any
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
struct StatemapInputTag {
    state: u32,  // state for this tag
    tag: String, // tag itself
}
