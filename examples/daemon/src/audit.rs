//! Homeledger audit destinations and trait-provider injection.

use overseerd::{component, injectable};

/// A durable destination for transaction audit records.
#[injectable]
pub trait AuditSink: Send + Sync {
    fn destination(&self) -> &'static str;
}

/// The primary local audit journal.
#[component(provide = dyn AuditSink, primary)]
pub struct JournalAudit;

impl AuditSink for JournalAudit {
    fn destination(&self) -> &'static str {
        "journal"
    }
}

/// A secondary archive audit destination.
#[component(provide = dyn AuditSink, qualifier = "archive", after = JournalAudit as dyn AuditSink)]
pub struct ArchiveAudit;

impl AuditSink for ArchiveAudit {
    fn destination(&self) -> &'static str {
        "archive"
    }
}
