#[path = "support.rs"]
mod support;

pub use support::edifact::{
    EdifactDeserialize, EdifactError, EdifactEvent, EdifactSegmentTag, EdifactSerialize, Element,
    EventEmitter, Segment, SegmentSliceExt};

extern crate self as edifact;

use edifact_derive::EdifactDeserialize as DeriveEdifactDeserialize;

// Identifiers belong in `element`, which resolves both coordinates.
#[derive(DeriveEdifactDeserialize)]
#[edifact(segment = "NAD")]
struct Message {
    #[edifact(element = 1, component = "3055")]
    agency: String}

fn main() {}
