#[path = "support.rs"]
mod support;

pub use support::edifact::{
    Element, EdifactDeserialize, EdifactError, EdifactEvent, EdifactSegmentTag,
    EdifactSerialize, EventEmitter, Segment,
    SegmentSliceExt};

extern crate self as edifact;

use edifact_derive::{
    EdifactDeserialize as DeriveEdifactDeserialize,
    EdifactSerialize as DeriveEdifactSerialize};

#[derive(DeriveEdifactSerialize, DeriveEdifactDeserialize)]
#[edifact(segment = "BGM")]
struct BgmSegment {
    #[edifact(element = 0)]
    code: String}

#[derive(DeriveEdifactSerialize)]
struct Message {
    #[edifact(element = 0)]
    bgm: BgmSegment}

fn main() {}