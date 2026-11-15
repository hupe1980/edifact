#[path = "support.rs"]
mod support;

pub use support::edifact::{
    Element, EdifactDeserialize, EdifactError, EdifactEvent, EdifactSegmentTag,
    EdifactSerialize, EventEmitter, Segment, SegmentSliceExt};

extern crate self as edifact;

use edifact_derive::EdifactDeserialize as DeriveEdifactDeserialize;

#[derive(DeriveEdifactDeserialize)]
#[edifact(segment = "NAD")]
struct NadSegment {
    #[edifact(qualifier = "MS")]
    party_id: String}

fn main() {}