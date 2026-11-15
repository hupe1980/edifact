#[path = "support.rs"]
mod support;

pub use support::edifact::{
    Element, EdifactDeserialize, EdifactError, EdifactEvent, EdifactSegmentTag,
    EdifactSerialize, EventEmitter, Segment, find_qualified_segment, find_segment};

extern crate self as edifact;

use edifact_derive::{EdifactDeserialize as DeriveEdifactDeserialize, EdifactSerialize as DeriveEdifactSerialize};

#[derive(DeriveEdifactSerialize, DeriveEdifactDeserialize)]
enum Message {
    Bgm}

fn main() {}