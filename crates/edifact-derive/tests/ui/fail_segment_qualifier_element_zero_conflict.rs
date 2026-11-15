#[path = "support.rs"]
mod support;

pub use support::edifact::{
    Element, EdifactDeserialize, EdifactError, EdifactEvent, EdifactSegmentTag,
    EdifactSerialize, EventEmitter, Segment, SegmentSliceExt};

extern crate self as edifact;

use edifact_derive::EdifactSerialize as DeriveEdifactSerialize;

#[derive(DeriveEdifactSerialize)]
#[edifact(segment = "RFF", qualifier = "AGI")]
struct RffSegment {
    #[edifact(element = 0)]
    qualifier: String,
    #[edifact(element = 1)]
    reference: String}

fn main() {}
