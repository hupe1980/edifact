#[path = "support.rs"]
mod support;

pub use support::edifact::{
    Element, EdifactDeserialize, EdifactError, EdifactEvent, EdifactSegmentTag,
    EdifactSerialize, EventEmitter, Segment, SegmentSliceExt};

extern crate self as edifact;

use edifact_derive::EdifactDeserialize as DeriveEdifactDeserialize;

// lowercase tag — must be rejected at compile time
#[derive(DeriveEdifactDeserialize)]
#[edifact(segment = "bgm")]
struct LowercaseTag {
    reference: String}

fn main() {}
