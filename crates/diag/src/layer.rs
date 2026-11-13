//! The ring layer, the subscriber's only output, and the span-less base subscriber it sits on.

use std::sync::Arc;

use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::Interest;
use tracing::{Event, Metadata, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

use crate::line::{FieldWriter, TextBuf, level_code};
use crate::raw;
use crate::ring::LogRing;

/// The span id the base subscriber gives every span, since it tracks none.
const SPAN_ID: u64 = 1;

/// Formats each enabled event into one slot of the log ring.
pub(crate) struct RingLayer {
    ring: Arc<LogRing>,
}

impl RingLayer {
    pub(crate) const fn new(ring: Arc<LogRing>) -> Self {
        Self { ring }
    }
}

impl<S: Subscriber> Layer<S> for RingLayer {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let metadata = event.metadata();
        let mut line = TextBuf::new();
        line.push_str(metadata.target());
        event.record(&mut FieldWriter { line: &mut line });
        let tick = raw::qpc_ticks();
        self.ring.push(tick, level_code(*metadata.level()), raw::thread_id(), line.finish());
    }
}

/// The subscriber under the layers. It enables everything and records nothing, so the layers decide.
///
/// It replaces tracing-subscriber's `Registry`, which pulls in `lazy_static` through `sharded-slab`,
/// a crate `deny.toml` bans.
pub(crate) struct Base;

impl Subscriber for Base {
    fn register_callsite(&self, _metadata: &'static Metadata<'static>) -> Interest {
        Interest::always()
    }

    fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _span: &Attributes<'_>) -> Id {
        Id::from_u64(SPAN_ID)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, _event: &Event<'_>) {}

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}
