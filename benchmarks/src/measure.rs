//! A Criterion [`Measurement`] that reports **bytes allocated** instead of wall-clock time.
//!
//! Wall-clock benchmarks answer "how fast"; on shared CI runners their absolute numbers drift.
//! Allocation traffic, by contrast, is deterministic per input and runner-independent — so
//! measuring the bytes a build allocates turns memory cost into a stable, trendable metric. A bench
//! configured with [`AllocBytes`] reports each routine's per-iteration heap traffic, sampled from
//! the [`TrackingAllocator`](crate::alloc::TrackingAllocator) the bench installs globally.
//!
//! ```ignore
//! use criterion::{Criterion, criterion_group, criterion_main};
//! use overseerd_benchmarks::{alloc::TrackingAllocator, measure::AllocBytes};
//!
//! #[global_allocator]
//! static GLOBAL: TrackingAllocator = TrackingAllocator;
//!
//! fn my_bench(c: &mut Criterion<AllocBytes>) { /* ... */ }
//!
//! criterion_group! {
//!     name = benches;
//!     config = Criterion::default().with_measurement(AllocBytes);
//!     targets = my_bench
//! }
//! criterion_main!(benches);
//! ```

use criterion::Throughput;
use criterion::measurement::{Measurement, ValueFormatter};

use crate::alloc::bytes_allocated;

/// A Criterion measurement whose value is the number of bytes allocated between `start` and `end`,
/// read from the global [`TrackingAllocator`](crate::alloc::TrackingAllocator).
pub struct AllocBytes;

impl Measurement for AllocBytes {
    type Intermediate = u64;
    type Value = u64;

    fn start(&self) -> Self::Intermediate {
        bytes_allocated()
    }

    fn end(&self, start: Self::Intermediate) -> Self::Value {
        bytes_allocated().saturating_sub(start)
    }

    fn add(&self, v1: &Self::Value, v2: &Self::Value) -> Self::Value {
        v1 + v2
    }

    fn zero(&self) -> Self::Value {
        0
    }

    fn to_f64(&self, value: &Self::Value) -> f64 {
        *value as f64
    }

    fn formatter(&self) -> &dyn ValueFormatter {
        &ByteFormatter
    }
}

/// Formats byte counts with binary SI-ish units, so a report reads "3.5 KiB" rather than "3584".
struct ByteFormatter;

impl ByteFormatter {
    /// Picks the divisor and unit label for a typical byte magnitude.
    fn unit_for(typical: f64) -> (f64, &'static str) {
        const KIB: f64 = 1024.0;
        const MIB: f64 = KIB * 1024.0;
        const GIB: f64 = MIB * 1024.0;

        if typical < KIB {
            (1.0, "B")
        } else if typical < MIB {
            (KIB, "KiB")
        } else if typical < GIB {
            (MIB, "MiB")
        } else {
            (GIB, "GiB")
        }
    }
}

impl ValueFormatter for ByteFormatter {
    fn scale_values(&self, typical_value: f64, values: &mut [f64]) -> &'static str {
        let (divisor, unit) = Self::unit_for(typical_value);

        for value in values.iter_mut() {
            *value /= divisor;
        }

        unit
    }

    fn scale_throughputs(
        &self,
        typical_value: f64,
        throughput: &Throughput,
        values: &mut [f64],
    ) -> &'static str {
        // "Bytes allocated per element processed" is the only throughput reading that makes sense
        // for a memory measurement; convert to bytes-per-element, then scale like plain bytes.
        let per_element = match throughput {
            Throughput::Bytes(n) | Throughput::BytesDecimal(n) | Throughput::Elements(n) => {
                *n as f64
            }
            Throughput::Bits(n) => *n as f64,
            Throughput::ElementsAndBytes { elements, .. } => *elements as f64,
        }
        .max(1.0);

        for value in values.iter_mut() {
            *value /= per_element;
        }

        let (divisor, unit) = Self::unit_for(typical_value / per_element);

        for value in values.iter_mut() {
            *value /= divisor;
        }

        unit
    }

    fn scale_for_machines(&self, _values: &mut [f64]) -> &'static str {
        // Leave the raw byte counts unscaled for CSV consumers.
        "bytes"
    }
}
