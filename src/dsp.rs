//! MBC4 signal path (the maths of the JUCE 1.x `MultibandEngine`, unchanged).
//!
//! ```text
//! in → IN gain → split → [per band] detect → gain curve → lookahead → attack/release → mix → gain → solo/mute
//!                                                                                   → sum → OUT gain → out
//! ```
//!
//! Crossover: 4th-order Linkwitz-Riley tree with allpass compensation, so the
//! bands sum back flat in magnitude.
//!
//! Per band (stereo linked, feed-forward):
//!   1. detection   L = 10 log10 (max (xL², xR²))            [dB, instantaneous peak]
//!   2. gain curve  soft knee, ratio                        → reduction r ≥ 0 [dB]
//!   3. lookahead   r' = max of r over the next L samples, audio delayed by L
//!   4. ballistics  hold = max (r', release-smoothed hold);  gr = attack-smoothed hold
//!   5. output      gain = ((1 − mix) + mix · 10^(−gr/20)) · makeup

use crate::shared::Shared;
use std::sync::atomic::Ordering::{Relaxed, Release};

pub const NUM_BANDS: usize = 4;
pub const MAX_LOOKAHEAD_MS: f64 = 10.0;
pub const MAX_CH: usize = 2;
pub const XO_DEFAULTS: [f32; 3] = [120.0, 1000.0, 6000.0];
/// the host block is processed in chunks of at most this many samples
pub const CHUNK: usize = 512;
/// ignore silence while learning
const LEARN_FLOOR_DB: f32 = -60.0;

#[derive(Clone, Copy, Debug)]
pub struct BandSettings {
    pub thresh_db: f32,
    pub ratio: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub knee_db: f32,
    pub makeup_db: f32,
    /// band dry / wet (0 – 1)
    pub mix: f32,
    /// band passes through unprocessed
    pub bypass: bool,
    /// false when muted / not soloed
    pub audible: bool,
}

impl Default for BandSettings {
    fn default() -> Self {
        Self {
            thresh_db: -18.0,
            ratio: 2.0,
            attack_ms: 10.0,
            release_ms: 120.0,
            knee_db: 6.0,
            makeup_db: 0.0,
            mix: 1.0,
            bypass: false,
            audible: true,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub bands: [BandSettings; NUM_BANDS],
    pub xover: [f32; 3],
    pub in_db: f32,
    pub out_db: f32,
    /// global dry / wet (0 – 1)
    pub mix: f32,
    pub lookahead_ms: f32,
    /// host bypass: the input, delayed by the reported latency
    pub bypass: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            bands: [BandSettings::default(); NUM_BANDS],
            xover: XO_DEFAULTS,
            in_db: 0.0,
            out_db: 0.0,
            mix: 1.0,
            lookahead_ms: 0.0,
            bypass: false,
        }
    }
}

#[inline]
pub fn db_to_gain(db: f32) -> f32 {
    if db > -100.0 { 10f32.powf(db * 0.05) } else { 0.0 }
}

#[inline]
pub fn gain_to_db(g: f32, floor: f32) -> f32 {
    if g > 0.0 { (20.0 * g.log10()).max(floor) } else { floor }
}

#[inline]
fn power_to_db(p: f32) -> f32 {
    10.0 * (p + 1.0e-12).log10()
}

/// Static curve (dB in → gain change in dB, ≤ 0), also used by the editor.
#[inline]
pub fn compute_gr(x: f32, thr: f32, slope: f32, knee: f32, half_knee: f32) -> f32 {
    let over = x - thr;
    if over <= -half_knee {
        return 0.0;
    }
    if over < half_knee && knee > 0.0 {
        let t = over + half_knee;
        return slope * t * t / (2.0 * knee);
    }
    slope * over
}

#[inline]
fn snap_to_zero(x: &mut f32) {
    if !(*x < -1.0e-8 || *x > 1.0e-8) {
        *x = 0.0;
    }
}

// ───────────────────────────────── Linkwitz-Riley (JUCE's TPT form)

#[derive(Clone, Copy, Default)]
struct Lr4 {
    g: f32,
    h: f32,
    s1: [f32; MAX_CH],
    s2: [f32; MAX_CH],
    s3: [f32; MAX_CH],
    s4: [f32; MAX_CH],
}

const R2: f32 = std::f32::consts::SQRT_2;

impl Lr4 {
    fn set_cutoff(&mut self, fc: f32, sr: f64) {
        self.g = (std::f64::consts::PI * f64::from(fc) / sr).tan() as f32;
        self.h = (1.0 / (1.0 + f64::from(R2 * self.g) + f64::from(self.g * self.g))) as f32;
    }

    fn reset(&mut self) {
        self.s1 = [0.0; MAX_CH];
        self.s2 = [0.0; MAX_CH];
        self.s3 = [0.0; MAX_CH];
        self.s4 = [0.0; MAX_CH];
    }

    fn snap(&mut self) {
        for s in [&mut self.s1, &mut self.s2, &mut self.s3, &mut self.s4] {
            s.iter_mut().for_each(snap_to_zero);
        }
    }

    /// (low, high)
    #[inline]
    fn split(&mut self, c: usize, x: f32) -> (f32, f32) {
        let (g, h) = (self.g, self.h);
        let y_h = (x - (R2 + g) * self.s1[c] - self.s2[c]) * h;
        let y_b = g * y_h + self.s1[c];
        self.s1[c] = g * y_h + y_b;
        let y_l = g * y_b + self.s2[c];
        self.s2[c] = g * y_b + y_l;

        let y_h2 = (y_l - (R2 + g) * self.s3[c] - self.s4[c]) * h;
        let y_b2 = g * y_h2 + self.s3[c];
        self.s3[c] = g * y_h2 + y_b2;
        let y_l2 = g * y_b2 + self.s4[c];
        self.s4[c] = g * y_b2 + y_l2;

        (y_l2, y_l - R2 * y_b + y_h - y_l2)
    }

    #[inline]
    fn allpass(&mut self, c: usize, x: f32) -> f32 {
        let (g, h) = (self.g, self.h);
        let y_h = (x - (R2 + g) * self.s1[c] - self.s2[c]) * h;
        let y_b = g * y_h + self.s1[c];
        self.s1[c] = g * y_h + y_b;
        let y_l = g * y_b + self.s2[c];
        self.s2[c] = g * y_b + y_l;
        y_l - R2 * y_b + y_h
    }
}

// ───────────────────────────────── smoothing (JUCE `SmoothedValue`)

#[derive(Clone, Copy, Default)]
struct Smooth {
    cur: f32,
    target: f32,
    step: f32,
    countdown: i32,
    steps: i32,
    /// multiplicative (log-domain) instead of linear
    mult: bool,
}

impl Smooth {
    fn new(mult: bool, v: f32) -> Self {
        Self { cur: v, target: v, mult, ..Default::default() }
    }
    fn reset(&mut self, sr: f64, secs: f64) {
        self.steps = (secs * sr).floor() as i32;
        self.set_now(self.target);
    }
    fn set_now(&mut self, v: f32) {
        self.cur = v;
        self.target = v;
        self.countdown = 0;
    }
    fn set_target(&mut self, v: f32) {
        if v == self.target {
            return;
        }
        if self.steps <= 0 {
            self.set_now(v);
            return;
        }
        self.countdown = self.steps;
        self.target = v;
        self.step = if self.mult {
            ((self.target.abs().ln() - self.cur.abs().ln()) / self.countdown as f32).exp()
        } else {
            (self.target - self.cur) / self.countdown as f32
        };
    }
    fn smoothing(&self) -> bool {
        self.countdown > 0
    }
    #[inline]
    fn next(&mut self) -> f32 {
        if self.countdown <= 0 {
            return self.target;
        }
        self.countdown -= 1;
        if self.countdown > 0 {
            if self.mult { self.cur *= self.step } else { self.cur += self.step }
        } else {
            self.cur = self.target;
        }
        self.cur
    }
    fn skip(&mut self, n: i32) {
        if n >= self.countdown {
            self.set_now(self.target);
            return;
        }
        if self.mult { self.cur *= self.step.powi(n) } else { self.cur += self.step * n as f32 }
        self.countdown -= n;
    }
}

// ───────────────────────────────── lookahead

/// Running maximum over the last `window` values (monotonic queue, O(1) amortised).
#[derive(Default)]
struct SlidingMax {
    val: Vec<f32>,
    idx: Vec<i64>,
    head: usize,
    size: usize,
    t: i64,
}

impl SlidingMax {
    fn prepare(&mut self, capacity: usize) {
        let cap = capacity.max(1);
        self.val = vec![0.0; cap];
        self.idx = vec![0; cap];
        self.clear();
    }
    fn clear(&mut self) {
        self.head = 0;
        self.size = 0;
        self.t = 0;
    }
    #[inline]
    fn push(&mut self, v: f32, window: usize) -> f32 {
        let cap = self.val.len();
        // drop smaller values from the back, they can never be the maximum again
        while self.size > 0 && self.val[(self.head + self.size - 1) % cap] <= v {
            self.size -= 1;
        }
        let back = (self.head + self.size) % cap;
        self.val[back] = v;
        self.idx[back] = self.t;
        self.size += 1;
        // drop values that left the window from the front
        while self.idx[self.head] <= self.t - window as i64 {
            self.head = (self.head + 1) % cap;
            self.size -= 1;
        }
        self.t += 1;
        self.val[self.head]
    }
}

#[derive(Default)]
struct Lookahead {
    /// MAX_CH × (max lookahead + 1)
    delay: Vec<f32>,
    write: usize,
    window: SlidingMax,
}

#[derive(Clone, Copy, Default)]
struct BandState {
    gr: f32,
    hold: f32,
}

#[derive(Clone, Copy, Default)]
struct Coeffs {
    thr: f32,
    slope: f32,
    knee: f32,
    half_knee: f32,
    att: f32,
    rel: f32,
    makeup: f32,
    mix: f32,
    active: bool,
}

// ───────────────────────────────── the 4-band compressor

pub struct Engine {
    sr: f64,
    split: [Lr4; 3],
    allpass: [Lr4; 3],
    band_buf: Box<[[[f32; CHUNK]; MAX_CH]; NUM_BANDS]>,
    state: [BandState; NUM_BANDS],
    look: [Lookahead; NUM_BANDS],
    x_smooth: [Smooth; 3],
    x_target: [f32; 3],
    band_gain: [Smooth; NUM_BANDS],
    max_lookahead: usize,
    lookahead: usize,
    /// gain-match accumulators (audio thread only)
    learn_sum: [f64; NUM_BANDS],
    learn_count: [u64; NUM_BANDS],
    /// smallest gain change of each band since `begin_block` (dB, ≤ 0)
    gr_acc: [f32; NUM_BANDS],
}

impl Default for Engine {
    fn default() -> Self {
        let mut e = Self {
            sr: 44100.0,
            split: [Lr4::default(); 3],
            allpass: [Lr4::default(); 3],
            band_buf: Box::new([[[0.0; CHUNK]; MAX_CH]; NUM_BANDS]),
            state: [BandState::default(); NUM_BANDS],
            look: Default::default(),
            x_smooth: XO_DEFAULTS.map(|f| Smooth::new(true, f)),
            x_target: XO_DEFAULTS,
            band_gain: [Smooth::new(false, 1.0); NUM_BANDS],
            max_lookahead: 0,
            lookahead: 0,
            learn_sum: [0.0; NUM_BANDS],
            learn_count: [0; NUM_BANDS],
            gr_acc: [0.0; NUM_BANDS],
        };
        e.prepare(44100.0);
        e
    }
}

impl Engine {
    /// Allocates; not for the audio thread.
    pub fn prepare(&mut self, sample_rate: f64) {
        self.sr = sample_rate;
        for x in &mut self.x_smooth {
            x.reset(sample_rate, 0.06);
        }
        for g in &mut self.band_gain {
            g.reset(sample_rate, 0.02);
        }
        self.max_lookahead = (MAX_LOOKAHEAD_MS * 0.001 * sample_rate).ceil() as usize;
        for la in &mut self.look {
            la.delay = vec![0.0; MAX_CH * (self.max_lookahead + 1)];
            la.window.prepare(self.max_lookahead + 2);
        }
        self.lookahead = self.lookahead.min(self.max_lookahead);
        self.reset();
    }

    pub fn reset(&mut self) {
        for f in self.split.iter_mut().chain(self.allpass.iter_mut()) {
            f.reset();
        }
        self.state = [BandState::default(); NUM_BANDS];
        for i in 0..3 {
            self.x_smooth[i].set_now(self.x_target[i]);
        }
        for g in &mut self.band_gain {
            g.set_now(1.0);
        }
        self.clear_lookahead();
        self.apply_crossovers();
    }

    pub fn set_crossovers(&mut self, f: [f32; 3]) {
        // keep the splits ordered with at least ~1/4 octave between them
        let nyq = (self.sr * 0.45) as f32;
        let f1 = f[0].clamp(20.0, nyq / 1.4);
        let f2 = f[1].clamp(f1 * 1.19, nyq / 1.19);
        let f3 = f[2].clamp(f2 * 1.19, nyq);
        self.x_target = [f1, f2, f3];
        for i in 0..3 {
            self.x_smooth[i].set_target(self.x_target[i]);
        }
    }

    /// Lookahead in samples (= added latency); clears the delay lines when it changes.
    pub fn set_lookahead(&mut self, samples: usize) {
        let s = samples.min(self.max_lookahead);
        if s != self.lookahead {
            self.lookahead = s;
            self.clear_lookahead();
        }
    }

    pub fn lookahead(&self) -> usize {
        self.lookahead
    }

    pub fn max_lookahead(&self) -> usize {
        self.max_lookahead
    }

    /// Start of a host block: the GR meters collect from here.
    pub fn begin_block(&mut self) {
        self.gr_acc = [0.0; NUM_BANDS];
    }

    /// End of a host block: publish the GR meters.
    pub fn publish(&self, sh: &Shared) {
        for (b, gr) in self.gr_acc.iter().enumerate() {
            sh.set_gr(b, *gr);
        }
    }

    /// One chunk (`n ≤ CHUNK`) of `nch` channels, in place.
    pub fn process(&mut self, io: &mut [[f32; CHUNK]; MAX_CH], nch: usize, n: usize, bands: &[BandSettings; NUM_BANDS], mix: f32, sh: &Shared) {
        let ch = nch.min(MAX_CH);
        if n == 0 || ch == 0 {
            return;
        }
        let sr = self.sr as f32;

        // ── per-block band coefficients
        let mut c = [Coeffs::default(); NUM_BANDS];
        for (b, s) in bands.iter().enumerate() {
            let k = &mut c[b];
            k.thr = s.thresh_db;
            k.slope = 1.0 / s.ratio.max(1.0) - 1.0;
            k.knee = s.knee_db.max(0.0);
            k.half_knee = k.knee * 0.5;
            k.att = (-1.0 / (s.attack_ms.max(0.01) * 0.001 * sr)).exp();
            k.rel = (-1.0 / (s.release_ms.max(1.0) * 0.001 * sr)).exp();
            k.makeup = s.makeup_db;
            k.mix = (s.mix * mix.clamp(0.0, 1.0)).clamp(0.0, 1.0);
            k.active = !s.bypass;
            self.band_gain[b].set_target(if s.audible { 1.0 } else { 0.0 });
        }

        // ── split into bands (sub-blocks so crossover moves stay smooth)
        const SUB: usize = 32;
        let mut start = 0;
        while start < n {
            let len = SUB.min(n - start);
            if self.x_smooth.iter().any(Smooth::smoothing) {
                for x in &mut self.x_smooth {
                    x.skip(len as i32);
                }
                self.apply_crossovers();
            }
            for c0 in 0..ch {
                for i in start..start + len {
                    let (low, rest) = self.split[0].split(c0, io[c0][i]);
                    let (mid, rest2) = self.split[1].split(c0, rest);
                    let (hi_mid, high) = self.split[2].split(c0, rest2);
                    // phase compensation for the lower bands
                    let low = self.allpass[0].allpass(c0, low); // @ f2
                    let low = self.allpass[1].allpass(c0, low); // @ f3
                    let mid = self.allpass[2].allpass(c0, mid); // @ f3
                    self.band_buf[0][c0][i] = low;
                    self.band_buf[1][c0][i] = mid;
                    self.band_buf[2][c0][i] = hi_mid;
                    self.band_buf[3][c0][i] = high;
                }
            }
            start += len;
        }

        // ── per band dynamics + sum
        for row in io.iter_mut().take(ch) {
            row[..n].fill(0.0);
        }
        let delay_size = self.max_lookahead + 1;
        let lookahead = self.lookahead;

        for (b, k) in c.iter().enumerate() {
            let st = &mut self.state[b];
            let la = &mut self.look[b];
            let g_sm = &mut self.band_gain[b];
            let buf = &self.band_buf[b];
            let mut min_gr = 0.0f32;
            let makeup_lin = db_to_gain(k.makeup);

            // gain-match learning: average the effective gain change while the band has signal
            let ln = &sh.learn[b];
            if ln.reset_request.swap(false, Relaxed) {
                self.learn_sum[b] = 0.0;
                self.learn_count[b] = 0;
            }
            let learning = ln.active.load(Relaxed) && k.active;

            for i in 0..n {
                // 1. detection (linked peak, on the undelayed band signal)
                let mut pk2 = 0.0f32;
                for row in buf.iter().take(ch) {
                    pk2 = pk2.max(row[i] * row[i]);
                }
                let lvl_db = power_to_db(pk2);

                // 2. gain computer → reduction (≥ 0 dB)
                let red = if k.active { -compute_gr(lvl_db, k.thr, k.slope, k.knee, k.half_knee) } else { 0.0 };

                // 3. lookahead: worst reduction over the next L samples
                let red_la = la.window.push(red, lookahead + 1);

                // 4. ballistics (smooth decoupled peak detector)
                let mut gain = 1.0;
                if k.active {
                    st.hold = red_la.max(k.rel * st.hold + (1.0 - k.rel) * red_la);
                    st.gr = st.hold + k.att * (st.gr - st.hold); // stored as a positive reduction
                    min_gr = min_gr.min(-st.gr);

                    // 5. output gain
                    let wet = db_to_gain(-st.gr);
                    let comp = (1.0 - k.mix) + k.mix * wet; // effective band gain change
                    gain = comp * makeup_lin;

                    if learning && lvl_db > LEARN_FLOOR_DB {
                        self.learn_sum[b] += f64::from(gain_to_db(comp, -60.0));
                        self.learn_count[b] += 1;
                    }
                } else {
                    st.gr = 0.0;
                    st.hold = 0.0;
                }

                gain *= g_sm.next();

                // delay the audio by the lookahead and apply the gain to it
                let w = la.write;
                let r = if w < lookahead { w + delay_size - lookahead } else { w - lookahead };
                for (c0, row) in io.iter_mut().enumerate().take(ch) {
                    let line = &mut la.delay[c0 * delay_size..(c0 + 1) * delay_size];
                    line[w] = buf[c0][i];
                    row[i] += line[r] * gain;
                }
                la.write = if w + 1 == delay_size { 0 } else { w + 1 };
            }

            self.gr_acc[b] = self.gr_acc[b].min(min_gr);
            ln.sum.store(self.learn_sum[b].to_bits(), Relaxed);
            ln.count.store(self.learn_count[b], Release);
        }

        for f in self.split.iter_mut().chain(self.allpass.iter_mut()) {
            f.snap();
        }
    }

    fn apply_crossovers(&mut self) {
        let [f1, f2, f3] = [self.x_smooth[0].cur, self.x_smooth[1].cur, self.x_smooth[2].cur];
        self.split[0].set_cutoff(f1, self.sr);
        self.split[1].set_cutoff(f2, self.sr);
        self.split[2].set_cutoff(f3, self.sr);
        self.allpass[0].set_cutoff(f2, self.sr);
        self.allpass[1].set_cutoff(f3, self.sr);
        self.allpass[2].set_cutoff(f3, self.sr);
    }

    fn clear_lookahead(&mut self) {
        for la in &mut self.look {
            la.delay.fill(0.0);
            la.write = 0;
            la.window.clear();
        }
    }
}

// ───────────────────────────────── whole plugin path

/// IN / OUT gain, meters, analyser feed and host bypass around the [`Engine`].
pub struct Processor {
    pub engine: Engine,
    sr: f64,
    in_gain: Smooth,
    out_gain: Smooth,
    /// host bypass keeps the same latency as the processed path
    bypass_delay: Vec<f32>,
    bypass_write: usize,
    latency: usize,
    pre_head: usize,
    post_head: usize,
    in_pk: f32,
    out_pk: f32,
}

impl Default for Processor {
    fn default() -> Self {
        let mut p = Self {
            engine: Engine::default(),
            sr: 44100.0,
            in_gain: Smooth::new(false, 1.0),
            out_gain: Smooth::new(false, 1.0),
            bypass_delay: Vec::new(),
            bypass_write: 0,
            latency: 0,
            pre_head: 0,
            post_head: 0,
            in_pk: 0.0,
            out_pk: 0.0,
        };
        p.prepare(44100.0, &Settings::default(), &Shared::default());
        p
    }
}

impl Processor {
    /// Allocates; not for the audio thread.
    pub fn prepare(&mut self, sample_rate: f64, set: &Settings, sh: &Shared) {
        self.sr = sample_rate;
        self.engine.prepare(sample_rate);
        self.engine.set_crossovers(set.xover);
        self.engine.reset();
        self.in_gain.reset(sample_rate, 0.03);
        self.out_gain.reset(sample_rate, 0.03);
        self.in_gain.set_now(db_to_gain(set.in_db));
        self.out_gain.set_now(db_to_gain(set.out_db));
        self.bypass_delay = vec![0.0; MAX_CH * (self.engine.max_lookahead() + 1)];
        self.bypass_write = 0;
        self.update_lookahead(set.lookahead_ms);
        self.pre_head = sh.pre.head();
        self.post_head = sh.post.head();
    }

    /// Latency in samples (= the lookahead), reported to the host.
    pub fn latency(&self) -> usize {
        self.latency
    }

    fn update_lookahead(&mut self, ms: f32) {
        let samples = (f64::from(ms) * 0.001 * self.sr).round().max(0.0) as usize;
        self.engine.set_lookahead(samples);
        self.latency = self.engine.lookahead();
    }

    /// Start of a host block: block-rate settings, latency, meters.
    pub fn begin_block(&mut self, set: &Settings) {
        if set.bypass {
            return;
        }
        self.engine.set_crossovers(set.xover);
        self.update_lookahead(set.lookahead_ms);
        self.in_gain.set_target(db_to_gain(set.in_db));
        self.out_gain.set_target(db_to_gain(set.out_db));
        self.engine.begin_block();
        self.in_pk = 0.0;
        self.out_pk = 0.0;
    }

    /// One chunk (`len ≤ CHUNK`) of `nch` channels, in place.
    pub fn process(&mut self, work: &mut [[f32; CHUNK]; MAX_CH], nch: usize, len: usize, set: &Settings, sh: &Shared) {
        let nch = nch.min(MAX_CH);
        if nch == 0 || len == 0 {
            return;
        }
        if set.bypass {
            self.process_bypassed(work, nch, len);
            return;
        }

        for i in 0..len {
            let g = self.in_gain.next();
            for row in work.iter_mut().take(nch) {
                row[i] *= g;
            }
        }
        self.in_pk = self.in_pk.max(peak(work, nch, len));
        push_mono(&sh.pre, &mut self.pre_head, work, nch, len);

        self.engine.process(work, nch, len, &set.bands, set.mix, sh);

        for i in 0..len {
            let g = self.out_gain.next();
            for row in work.iter_mut().take(nch) {
                row[i] *= g;
            }
        }
        self.out_pk = self.out_pk.max(peak(work, nch, len));
        push_mono(&sh.post, &mut self.post_head, work, nch, len);
    }

    /// End of a host block: publish meters and analyser samples.
    pub fn end_block(&mut self, set: &Settings, sh: &Shared) {
        if set.bypass {
            return;
        }
        self.engine.publish(sh);
        sh.pre.publish(self.pre_head);
        sh.post.publish(self.post_head);
        sh.set_peaks(gain_to_db(self.in_pk, -100.0), gain_to_db(self.out_pk, -100.0));
    }

    /// A whole block of separate channel slices (tests, offline use).
    pub fn run(&mut self, chans: &mut [&mut [f32]], set: &Settings, sh: &Shared) {
        let nch = chans.len().min(MAX_CH);
        let n = chans.first().map_or(0, |c| c.len());
        let mut work = [[0.0f32; CHUNK]; MAX_CH];
        self.begin_block(set);
        let mut pos = 0;
        while pos < n {
            let len = CHUNK.min(n - pos);
            for c in 0..nch {
                work[c][..len].copy_from_slice(&chans[c][pos..pos + len]);
            }
            self.process(&mut work, nch, len, set, sh);
            for c in 0..nch {
                chans[c][pos..pos + len].copy_from_slice(&work[c][..len]);
            }
            pos += len;
        }
        self.end_block(set, sh);
    }

    /// The dry signal delayed by the reported latency, so bypassing stays time-aligned.
    fn process_bypassed(&mut self, work: &mut [[f32; CHUNK]; MAX_CH], nch: usize, len: usize) {
        let lat = self.latency;
        let size = self.engine.max_lookahead() + 1;
        if lat == 0 || size <= lat {
            return;
        }
        for i in 0..len {
            let r = (self.bypass_write + size - lat) % size;
            for (c, row) in work.iter_mut().enumerate().take(nch) {
                let line = &mut self.bypass_delay[c * size..(c + 1) * size];
                line[self.bypass_write] = row[i];
                row[i] = line[r];
            }
            self.bypass_write = (self.bypass_write + 1) % size;
        }
    }
}

fn peak(work: &[[f32; CHUNK]; MAX_CH], nch: usize, len: usize) -> f32 {
    work.iter().take(nch).flat_map(|r| r[..len].iter()).fold(0.0f32, |m, v| m.max(v.abs()))
}

fn push_mono(ring: &crate::shared::Ring, head: &mut usize, work: &[[f32; CHUNK]; MAX_CH], nch: usize, len: usize) {
    for i in 0..len {
        let v = if nch == 1 { work[0][i] } else { 0.5 * (work[0][i] + work[1][i]) };
        ring.push(head, v);
    }
}
