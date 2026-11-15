#[path = "support.rs"]
mod support;

pub use support::edifact::{EdifactError, EdifactEvent, EdifactSerialize, EventEmitter};

extern crate self as edifact;

use edifact_derive::EdifactSerialize as DeriveEdifactSerialize;

// Two fields claiming the same slot would silently drop one on serialize.
#[derive(DeriveEdifactSerialize)]
#[edifact(segment = "BGM")]
struct Bad {
    #[edifact(element = 1)]
    first: String,
    #[edifact(element = 1)]
    second: String}

fn main() {}
