//! Sensor-fusion pipeline — the default demo (#79).
//!
//! ```text
//! sensor_reader (pri 3) --Channel 0--> filter (pri 2) --Channel 1--> uart_logger (pri 1)
//!   every 10 ms               moving average               slow formatted output
//! ```
//!
//! - `sensor_reader` samples a simulated sensor every [`PERIOD`] ticks and
//!   publishes a raw reading with `try_send`, so it never blocks.  It
//!   measures its own *release latency* (ticks between its scheduled release
//!   and actually running) and counts *deadline misses* (finishing
//!   [`DEADLINE`] or more ticks after release).
//! - `filter` blocks on Channel 0, keeps a [`WINDOW`]-sample moving average,
//!   and forwards it on Channel 1.
//! - `uart_logger` blocks on Channel 1 and spends [`LOGGER_WORK`] ticks of
//!   CPU per line, plus a [`FLUSH_WORK`]-tick "flush" every
//!   [`FLUSH_EVERY`] lines — a stand-in for a slow, bursty output device.
//!
//! A flush lasts longer than the sensor period, so the sensor's next
//! releases fall *inside* it.  Because `sensor_reader` has the highest
//! effective priority it preempts the logger at each of those releases:
//! release latency stays at 0–1 tick and the miss counter at 0.  The logger
//! prints every flush window, and the boot test checks that samples released
//! inside a window were still on time.  On average the logger keeps up
//! (`FLUSH_EVERY * LOGGER_WORK + FLUSH_WORK` < `FLUSH_EVERY * PERIOD`), with
//! the channels absorbing the backlog.

use super::claim_stack;
use ferret::config::MAX_TASKS;
use ferret::generated::ccg_constants::PIPELINE_MAX_INHERITED_PRIORITIES;
use ferret::generated::pipeline_tasks::{FILTER, SENSOR_READER, UART_LOGGER};
use ferret::ipc::{ReceiverCapability, SenderCapability};
use ferret::memory::{self, register_task};
use ferret::sys::{self, LineBuf, SysError};

/// Sensor sampling period in ticks (1 tick = 1 ms).
pub const PERIOD: u32 = 10;
/// Relative deadline: a sample must be published within this many ticks of
/// its release.
pub const DEADLINE: u32 = PERIOD;
/// Moving-average window of the filter, in samples.
pub const WINDOW: usize = 4;
/// CPU ticks the logger spends per line, simulating a slow output device.
pub const LOGGER_WORK: u32 = 3;
/// CPU ticks of each periodic flush — longer than [`PERIOD`] on purpose.
pub const FLUSH_WORK: u32 = 25;
/// The logger flushes after every this many lines.
pub const FLUSH_EVERY: u32 = 5;
const _: () = assert!(FLUSH_WORK > PERIOD, "a flush must span a sensor release");
const _: () = assert!(FLUSH_EVERY * LOGGER_WORK + FLUSH_WORK < FLUSH_EVERY * PERIOD, "logger must keep up");
/// The logger prints a summary line every this many samples.
pub const SUMMARY_EVERY: u32 = 100;

/// One sample as it travels down the pipeline (16 bytes on both channels).
///
/// `sensor_reader` fills everything but `avg`; `filter` fills `avg`.  The
/// sensor's timing counters ride along so the logger can report them.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct Reading {
    seq: u32,
    /// Tick at which the sensor was scheduled to take this sample.
    release: u32,
    raw: u16,
    avg: u16,
    /// Ticks between `release` and the sensor actually running.
    latency: u8,
    /// Deadline misses so far (saturating).
    misses: u8,
    /// Samples dropped because Channel 0 was full (saturating).
    dropped: u16,
}

impl Reading {
    fn encode(&self) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[0..4].copy_from_slice(&self.seq.to_le_bytes());
        b[4..8].copy_from_slice(&self.release.to_le_bytes());
        b[8..10].copy_from_slice(&self.raw.to_le_bytes());
        b[10..12].copy_from_slice(&self.avg.to_le_bytes());
        b[12] = self.latency;
        b[13] = self.misses;
        b[14..16].copy_from_slice(&self.dropped.to_le_bytes());
        b
    }

    fn decode(b: &[u8; 16]) -> Reading {
        Reading {
            seq: u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            release: u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
            raw: u16::from_le_bytes([b[8], b[9]]),
            avg: u16::from_le_bytes([b[10], b[11]]),
            latency: b[12],
            misses: b[13],
            dropped: u16::from_le_bytes([b[14], b[15]]),
        }
    }
}

/// Deterministic simulated sensor: a triangle wave (0..=1000, 64-sample
/// period) plus ±15 of pseudo-random noise from a fixed-seed LCG.
fn sample(seq: u32, noise_state: &mut u32) -> u16 {
    let phase = seq % 64;
    let tri = if phase < 32 { phase * 1000 / 32 } else { (64 - phase) * 1000 / 32 };
    *noise_state = noise_state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    let noise = (*noise_state >> 16) % 31;
    (tri + noise).saturating_sub(15) as u16
}

/// `sensor_reader` — priority 3, holds UART1 (the simulated sensor).
pub fn sensor_reader() -> ! {
    let tx = SenderCapability::<0>;
    let mut noise = 0x1234_5678u32;
    let mut seq = 0u32;
    let mut misses = 0u8;
    let mut dropped = 0u16;
    let mut release = sys::now();
    loop {
        let start = sys::now();
        let reading = Reading {
            seq,
            release,
            raw: sample(seq, &mut noise),
            avg: 0,
            latency: start.wrapping_sub(release).min(255) as u8,
            misses,
            dropped,
        };
        if tx.try_send(&reading.encode()) == Err(SysError::WouldBlock) {
            dropped = dropped.saturating_add(1);
        }
        if sys::now().wrapping_sub(release) >= DEADLINE {
            misses = misses.saturating_add(1);
        }
        seq = seq.wrapping_add(1);
        release = release.wrapping_add(PERIOD);
        let now = sys::now();
        // Sleep until the next release; if it has already passed (overrun),
        // run again immediately and let the latency show it.
        if (release.wrapping_sub(now) as i32) > 0 {
            sys::sleep(release.wrapping_sub(now));
        }
    }
}

/// `filter` — priority 2: moving average over the last [`WINDOW`] samples.
pub fn filter() -> ! {
    super::probe_log_validation();
    let rx = ReceiverCapability::<0>;
    let tx = SenderCapability::<1>;
    let mut window = [0u16; WINDOW];
    let mut filled = 0usize;
    let mut buf = [0u8; 16];
    loop {
        if rx.receive(&mut buf).is_err() {
            continue;
        }
        let mut r = Reading::decode(&buf);
        window[r.seq as usize % WINDOW] = r.raw;
        filled = (filled + 1).min(WINDOW);
        r.avg = (window[..filled].iter().map(|&v| v as u32).sum::<u32>() / filled as u32) as u16;
        let _ = tx.send(&r.encode());
    }
}

/// Busy-wait for `ticks` ticks of wall time without yielding.
fn burn(ticks: u32) -> (u32, u32) {
    let start = sys::now();
    while sys::now() < start + ticks {}
    (start, sys::now())
}

/// `uart_logger` — priority 1, holds UART0; slow, bursty output.
pub fn uart_logger() -> ! {
    let rx = ReceiverCapability::<1>;
    let mut buf = [0u8; 16];
    let mut max_latency = 0u8;
    let mut count = 0u32;
    loop {
        if rx.receive(&mut buf).is_err() {
            continue;
        }
        let r = Reading::decode(&buf);
        burn(LOGGER_WORK);
        max_latency = max_latency.max(r.latency);
        count += 1;
        let _ = LineBuf::<112>::new()
            .str("[LOG] t=").u32(sys::now())
            .str(" seq=").u32(r.seq)
            .str(" release=").u32(r.release)
            .str(" raw=").u32(r.raw as u32)
            .str(" avg=").u32(r.avg as u32)
            .str(" latency=").u32(r.latency as u32)
            .str(" misses=").u32(r.misses as u32)
            .str(" dropped=").u32(r.dropped as u32)
            .str("\n")
            .log();
        if count % FLUSH_EVERY == 0 {
            // The sensor releases at least once during this window and must
            // preempt it each time.
            let (start, end) = burn(FLUSH_WORK);
            let _ = LineBuf::<64>::new()
                .str("[LOG] flush start=").u32(start)
                .str(" end=").u32(end)
                .str("\n")
                .log();
        }
        if count % SUMMARY_EVERY == 0 {
            let _ = LineBuf::<112>::new()
                .str("[LOG] summary: samples=").u32(count)
                .str(" max_latency=").u32(max_latency as u32)
                .str(" misses=").u32(r.misses as u32)
                .str(" dropped=").u32(r.dropped as u32)
                .str("\n")
                .log();
        }
    }
}

// Each stack fills its task's 4 KB region in TASKRAM (slot order).
#[link_section = ".task_ram.0"]
static mut STACK_SENSOR: memory::Stack<4096> = memory::Stack::new();
#[link_section = ".task_ram.1"]
static mut STACK_FILTER: memory::Stack<4096> = memory::Stack::new();
#[link_section = ".task_ram.2"]
static mut STACK_LOGGER: memory::Stack<4096> = memory::Stack::new();

/// Register the three pipeline stages in slot order and return their
/// precomputed MIP table.
pub fn register() -> &'static [u8; MAX_TASKS] {
    register_task(SENSOR_READER.into_descriptor(0, claim_stack(core::ptr::addr_of!(STACK_SENSOR) as usize), sensor_reader));
    register_task(FILTER.into_descriptor(1, claim_stack(core::ptr::addr_of!(STACK_FILTER) as usize), filter));
    register_task(UART_LOGGER.into_descriptor(2, claim_stack(core::ptr::addr_of!(STACK_LOGGER) as usize), uart_logger));
    &PIPELINE_MAX_INHERITED_PRIORITIES
}
