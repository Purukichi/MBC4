//! Audio thread ↔ editor: analyser samples, meters and the gain-match learning.
//!
//! Everything is lock-free. The analyser rings may tear while the editor reads
//! them - harmless for a display.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering::{Acquire, Relaxed, Release}};

use crate::dsp::NUM_BANDS;

pub const RING: usize = 1 << 14;

/// Mono sample ring (audio → editor). The writer keeps its own head and
/// publishes it once per block.
pub struct Ring {
    data: Box<[AtomicU32]>,
    write: AtomicUsize,
}

impl Default for Ring {
    fn default() -> Self {
        Self { data: (0..RING).map(|_| AtomicU32::new(0)).collect(), write: AtomicUsize::new(0) }
    }
}

impl Ring {
    #[inline]
    pub fn push(&self, w: &mut usize, v: f32) {
        self.data[*w % RING].store(v.to_bits(), Relaxed);
        *w = w.wrapping_add(1);
    }

    pub fn head(&self) -> usize {
        self.write.load(Relaxed)
    }

    pub fn publish(&self, w: usize) {
        self.write.store(w, Relaxed);
    }

    /// Editor: the newest `out.len()` samples, oldest first.
    pub fn latest(&self, out: &mut [f32]) {
        let n = out.len().min(RING);
        let start = self.write.load(Relaxed).wrapping_sub(n);
        for (i, o) in out.iter_mut().enumerate().take(n) {
            *o = f32::from_bits(self.data[start.wrapping_add(i) % RING].load(Relaxed));
        }
    }
}

/// Gain match: the audio thread averages a band's effective gain change while
/// `active`; the editor writes the compensation into the band's gain once.
#[derive(Default)]
pub struct Learn {
    pub active: AtomicBool,
    pub reset_request: AtomicBool,
    /// f64 bits
    pub sum: AtomicU64,
    pub count: AtomicU64,
}

pub struct Shared {
    /// input after IN gain / output after OUT gain, mono
    pub pre: Ring,
    pub post: Ring,
    pub sample_rate: AtomicU32,
    /// f32 bits: gain reduction per band (dB, <= 0), peaks (dB)
    gr: [AtomicU32; NUM_BANDS],
    in_peak: AtomicU32,
    out_peak: AtomicU32,
    pub learn: [Learn; NUM_BANDS],
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            pre: Ring::default(),
            post: Ring::default(),
            sample_rate: AtomicU32::new(48000),
            gr: Default::default(),
            in_peak: AtomicU32::new((-100.0f32).to_bits()),
            out_peak: AtomicU32::new((-100.0f32).to_bits()),
            learn: Default::default(),
        }
    }
}

impl Shared {
    pub fn set_gr(&self, band: usize, db: f32) {
        self.gr[band].store(db.to_bits(), Relaxed);
    }
    pub fn gr_db(&self, band: usize) -> f32 {
        f32::from_bits(self.gr[band].load(Relaxed))
    }
    pub fn set_peaks(&self, in_db: f32, out_db: f32) {
        self.in_peak.store(in_db.to_bits(), Relaxed);
        self.out_peak.store(out_db.to_bits(), Relaxed);
    }
    pub fn in_peak_db(&self) -> f32 {
        f32::from_bits(self.in_peak.load(Relaxed))
    }
    pub fn out_peak_db(&self) -> f32 {
        f32::from_bits(self.out_peak.load(Relaxed))
    }

    // ── gain match (editor)

    pub fn start_learning(&self, band: usize) {
        let l = &self.learn[band];
        l.reset_request.store(true, Relaxed);
        l.count.store(0, Relaxed);
        l.sum.store(0f64.to_bits(), Relaxed);
        l.active.store(true, Release);
    }
    pub fn stop_learning(&self, band: usize) {
        self.learn[band].active.store(false, Release);
    }
    pub fn is_learning(&self, band: usize) -> bool {
        self.learn[band].active.load(Acquire)
    }
    /// seconds of signal collected so far
    pub fn learned_seconds(&self, band: usize) -> f64 {
        self.learn[band].count.load(Acquire) as f64 / f64::from(self.sample_rate.load(Relaxed).max(1))
    }
    /// average effective gain change in dB (<= 0)
    pub fn learned_average_db(&self, band: usize) -> f32 {
        let l = &self.learn[band];
        let n = l.count.load(Acquire);
        if n == 0 { 0.0 } else { (f64::from_bits(l.sum.load(Relaxed)) / n as f64) as f32 }
    }
}
