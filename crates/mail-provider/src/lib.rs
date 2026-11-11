pub mod imap;
mod graph;
pub use graph::{GraphProvider, GraphCursor, GraphDeltaPage, GraphArrivalPage, GraphSubscription};
#[cfg(test)]
extern crate self as dakia_mail_provider;
pub mod mime;
pub mod provider;
pub mod smtp;
pub mod types;
pub use provider::{AccountProvider, MailProvider, MetadataLookup, SubmissionGate};
pub use types::*;

mod telemetry;

#[cfg(test)]
#[path = "../tests/common/mod.rs"]
mod test_common;
