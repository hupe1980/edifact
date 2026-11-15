#[path = "support.rs"]
mod support;

pub use support::edifact::{
    EdifactDeserialize, EdifactError, EdifactEvent, EdifactSegmentTag, EdifactSerialize, Element,
    EventEmitter, Segment, find_qualified_segment, find_segment};

extern crate self as edifact;

use edifact_derive::EdifactDeserialize as DeriveEdifactDeserialize;

// A data element identifier has nothing to resolve against without a layout.
#[derive(DeriveEdifactDeserialize)]
#[edifact(segment = "NAD")]
struct Message {
    #[edifact(element = "3035")]
    qualifier: String}

fn main() {}
