//! MBC4 - 4-band multiband compressor (truce).
//!
//! Signal path in [`dsp`], audio ↔ editor state in [`shared`], the painted
//! editor in `ui`.

use std::sync::Arc;
use truce::prelude::*;

pub mod dsp;
pub mod shared;
mod ui;

use dsp::{BandSettings, CHUNK, MAX_CH, NUM_BANDS, Processor, Settings};

#[derive(Params)]
pub struct Mbc4Params {
    // ── crossovers
    #[param(name = "Crossover 1", short_name = "XO1", range = "log(20, 20000)", default = 120.0, unit = "Hz", format = "fmt_hz", parse = "parse_hz")]
    pub xover1: FloatParam,
    #[param(name = "Crossover 2", short_name = "XO2", range = "log(20, 20000)", default = 1000.0, unit = "Hz", format = "fmt_hz", parse = "parse_hz")]
    pub xover2: FloatParam,
    #[param(name = "Crossover 3", short_name = "XO3", range = "log(20, 20000)", default = 6000.0, unit = "Hz", format = "fmt_hz", parse = "parse_hz")]
    pub xover3: FloatParam,

    // ── band A (Low). Attack / release default slower in the low bands (long periods:
    // fast gain changes would distort the waveform), faster up top.
    #[param(name = "Low Threshold", short_name = "A THS", group = "Low", range = "linear(-60, 0)", default = 0.0, unit = "dB", format = "fmt_db")]
    pub a_thresh: FloatParam,
    #[param(name = "Low Ratio", short_name = "A RAT", group = "Low", range = "skewed(1, 20, 0.375521)", default = 2.0, format = "fmt_ratio")]
    pub a_ratio: FloatParam,
    #[param(name = "Low Attack", short_name = "A ATK", group = "Low", range = "skewed(0.1, 300, 0.230888)", default = 25.0, unit = "ms", format = "fmt_ms")]
    pub a_attack: FloatParam,
    #[param(name = "Low Release", short_name = "A RLS", group = "Low", range = "skewed(5, 3000, 0.253742)", default = 250.0, unit = "ms", format = "fmt_ms")]
    pub a_release: FloatParam,
    #[param(name = "Low Knee", short_name = "A KNE", group = "Low", range = "linear(0, 24)", default = 6.0, unit = "dB", format = "fmt_db")]
    pub a_knee: FloatParam,
    #[param(name = "Low Gain", short_name = "A GAIN", group = "Low", range = "linear(-24, 24)", default = 0.0, unit = "dB", format = "fmt_db")]
    pub a_gain: FloatParam,
    #[param(name = "Low Mix", short_name = "A MIX", group = "Low", range = "linear(0, 100)", default = 100.0, unit = "%", format = "fmt_pct")]
    pub a_mix: FloatParam,
    #[param(name = "Low Solo", group = "Low")]
    pub a_solo: BoolParam,
    #[param(name = "Low Mute", group = "Low")]
    pub a_mute: BoolParam,
    #[param(name = "Low Bypass", group = "Low")]
    pub a_bypass: BoolParam,

    // ── band B (LowMid)
    #[param(name = "LowMid Threshold", short_name = "B THS", group = "LowMid", range = "linear(-60, 0)", default = 0.0, unit = "dB", format = "fmt_db")]
    pub b_thresh: FloatParam,
    #[param(name = "LowMid Ratio", short_name = "B RAT", group = "LowMid", range = "skewed(1, 20, 0.375521)", default = 2.0, format = "fmt_ratio")]
    pub b_ratio: FloatParam,
    #[param(name = "LowMid Attack", short_name = "B ATK", group = "LowMid", range = "skewed(0.1, 300, 0.230888)", default = 15.0, unit = "ms", format = "fmt_ms")]
    pub b_attack: FloatParam,
    #[param(name = "LowMid Release", short_name = "B RLS", group = "LowMid", range = "skewed(5, 3000, 0.253742)", default = 180.0, unit = "ms", format = "fmt_ms")]
    pub b_release: FloatParam,
    #[param(name = "LowMid Knee", short_name = "B KNE", group = "LowMid", range = "linear(0, 24)", default = 6.0, unit = "dB", format = "fmt_db")]
    pub b_knee: FloatParam,
    #[param(name = "LowMid Gain", short_name = "B GAIN", group = "LowMid", range = "linear(-24, 24)", default = 0.0, unit = "dB", format = "fmt_db")]
    pub b_gain: FloatParam,
    #[param(name = "LowMid Mix", short_name = "B MIX", group = "LowMid", range = "linear(0, 100)", default = 100.0, unit = "%", format = "fmt_pct")]
    pub b_mix: FloatParam,
    #[param(name = "LowMid Solo", group = "LowMid")]
    pub b_solo: BoolParam,
    #[param(name = "LowMid Mute", group = "LowMid")]
    pub b_mute: BoolParam,
    #[param(name = "LowMid Bypass", group = "LowMid")]
    pub b_bypass: BoolParam,

    // ── band C (HiMid)
    #[param(name = "HiMid Threshold", short_name = "C THS", group = "HiMid", range = "linear(-60, 0)", default = 0.0, unit = "dB", format = "fmt_db")]
    pub c_thresh: FloatParam,
    #[param(name = "HiMid Ratio", short_name = "C RAT", group = "HiMid", range = "skewed(1, 20, 0.375521)", default = 2.0, format = "fmt_ratio")]
    pub c_ratio: FloatParam,
    #[param(name = "HiMid Attack", short_name = "C ATK", group = "HiMid", range = "skewed(0.1, 300, 0.230888)", default = 8.0, unit = "ms", format = "fmt_ms")]
    pub c_attack: FloatParam,
    #[param(name = "HiMid Release", short_name = "C RLS", group = "HiMid", range = "skewed(5, 3000, 0.253742)", default = 120.0, unit = "ms", format = "fmt_ms")]
    pub c_release: FloatParam,
    #[param(name = "HiMid Knee", short_name = "C KNE", group = "HiMid", range = "linear(0, 24)", default = 6.0, unit = "dB", format = "fmt_db")]
    pub c_knee: FloatParam,
    #[param(name = "HiMid Gain", short_name = "C GAIN", group = "HiMid", range = "linear(-24, 24)", default = 0.0, unit = "dB", format = "fmt_db")]
    pub c_gain: FloatParam,
    #[param(name = "HiMid Mix", short_name = "C MIX", group = "HiMid", range = "linear(0, 100)", default = 100.0, unit = "%", format = "fmt_pct")]
    pub c_mix: FloatParam,
    #[param(name = "HiMid Solo", group = "HiMid")]
    pub c_solo: BoolParam,
    #[param(name = "HiMid Mute", group = "HiMid")]
    pub c_mute: BoolParam,
    #[param(name = "HiMid Bypass", group = "HiMid")]
    pub c_bypass: BoolParam,

    // ── band D (High)
    #[param(name = "High Threshold", short_name = "D THS", group = "High", range = "linear(-60, 0)", default = 0.0, unit = "dB", format = "fmt_db")]
    pub d_thresh: FloatParam,
    #[param(name = "High Ratio", short_name = "D RAT", group = "High", range = "skewed(1, 20, 0.375521)", default = 2.0, format = "fmt_ratio")]
    pub d_ratio: FloatParam,
    #[param(name = "High Attack", short_name = "D ATK", group = "High", range = "skewed(0.1, 300, 0.230888)", default = 3.0, unit = "ms", format = "fmt_ms")]
    pub d_attack: FloatParam,
    #[param(name = "High Release", short_name = "D RLS", group = "High", range = "skewed(5, 3000, 0.253742)", default = 80.0, unit = "ms", format = "fmt_ms")]
    pub d_release: FloatParam,
    #[param(name = "High Knee", short_name = "D KNE", group = "High", range = "linear(0, 24)", default = 6.0, unit = "dB", format = "fmt_db")]
    pub d_knee: FloatParam,
    #[param(name = "High Gain", short_name = "D GAIN", group = "High", range = "linear(-24, 24)", default = 0.0, unit = "dB", format = "fmt_db")]
    pub d_gain: FloatParam,
    #[param(name = "High Mix", short_name = "D MIX", group = "High", range = "linear(0, 100)", default = 100.0, unit = "%", format = "fmt_pct")]
    pub d_mix: FloatParam,
    #[param(name = "High Solo", group = "High")]
    pub d_solo: BoolParam,
    #[param(name = "High Mute", group = "High")]
    pub d_mute: BoolParam,
    #[param(name = "High Bypass", group = "High")]
    pub d_bypass: BoolParam,

    // ── global
    #[param(name = "Input", short_name = "IN", range = "linear(-36, 36)", default = 0.0, unit = "dB", format = "fmt_db")]
    pub in_gain: FloatParam,
    #[param(name = "Output", short_name = "OUT", range = "linear(-36, 36)", default = 0.0, unit = "dB", format = "fmt_db")]
    pub out_gain: FloatParam,
    #[param(name = "Lookahead", short_name = "LOOK", range = "linear(0, 10)", default = 0.0, unit = "ms", format = "fmt_ms")]
    pub lookahead: FloatParam,
    #[param(name = "Mix", short_name = "MIX", range = "linear(0, 100)", default = 100.0, unit = "%", format = "fmt_pct")]
    pub mix: FloatParam,
    /// Host bypass (the DAW's plug-in on / off): the input, delayed by the latency.
    #[param(name = "Bypass", short_name = "BYP", flags = "automatable | bypass")]
    pub bypass: BoolParam,

    /// Editor size, as the width of the folded layout in logical points, remembered with the project.
    #[persist = "ui_width"]
    pub ui_width: AtomicCell<u32>,
    /// The INPUT / OUTPUT panel on the right: 0 folded (default), 1 open.
    #[persist = "io_open"]
    pub io_open: AtomicCell<u32>,

    /// Meters, analyser feed and gain match.
    #[skip]
    pub shared: Arc<shared::Shared>,
}

/// "-24", "2.5" (one decimal at most, never "-0")
pub fn number_text(v: f64) -> String {
    let r = (v * 10.0).round() / 10.0;
    if r.abs() < 0.05 {
        "0".into()
    } else if (r - r.round()).abs() < 0.01 {
        format!("{:.0}", r.round())
    } else {
        format!("{r:.1}")
    }
}

pub fn ms_text(v: f64) -> String {
    if v < 10.0 { format!("{:.1}", (v * 10.0).round() / 10.0) } else { format!("{:.0}", v.round()) }
}

pub fn hz_short(f: f64) -> String {
    if f >= 1000.0 { format!("{:.*}k", if f >= 10000.0 { 1 } else { 2 }, f / 1000.0) } else { format!("{:.0}", f.round()) }
}

/// accepts "1200", "1.2k", "1.2 kHz", "850hz"
pub fn hz_from_text(t: &str) -> Option<f64> {
    let s = t.trim().to_ascii_lowercase().replace(' ', "").replace("hz", "");
    let kilo = s.contains('k');
    let v: f64 = s.replace('k', "").parse().ok()?;
    Some(if kilo { v * 1000.0 } else { v })
}

impl Mbc4Params {
    fn fmt_hz(&self, v: f64) -> String {
        if v >= 1000.0 { format!("{:.*} kHz", if v >= 10000.0 { 1 } else { 2 }, v / 1000.0) } else { format!("{:.0} Hz", v.round()) }
    }
    fn parse_hz(&self, t: &str) -> Option<f64> {
        hz_from_text(t)
    }
    fn fmt_db(&self, v: f64) -> String {
        format!("{v:.1} dB")
    }
    fn fmt_ms(&self, v: f64) -> String {
        format!("{} ms", ms_text(v))
    }
    fn fmt_ratio(&self, v: f64) -> String {
        if v >= 19.95 { "20:1".into() } else if v < 10.0 { format!("{v:.1}:1") } else { format!("{v:.0}:1") }
    }
    fn fmt_pct(&self, v: f64) -> String {
        format!("{:.0} %", v.round())
    }

    /// Plain-value snapshot for the DSP.
    pub fn settings(&self) -> Settings {
        let solo = [self.a_solo.value(), self.b_solo.value(), self.c_solo.value(), self.d_solo.value()];
        let mute = [self.a_mute.value(), self.b_mute.value(), self.c_mute.value(), self.d_mute.value()];
        let any_solo = solo.iter().any(|s| *s);
        #[rustfmt::skip]
        let p = [
            (&self.a_thresh, &self.a_ratio, &self.a_attack, &self.a_release, &self.a_knee, &self.a_gain, &self.a_mix, &self.a_bypass),
            (&self.b_thresh, &self.b_ratio, &self.b_attack, &self.b_release, &self.b_knee, &self.b_gain, &self.b_mix, &self.b_bypass),
            (&self.c_thresh, &self.c_ratio, &self.c_attack, &self.c_release, &self.c_knee, &self.c_gain, &self.c_mix, &self.c_bypass),
            (&self.d_thresh, &self.d_ratio, &self.d_attack, &self.d_release, &self.d_knee, &self.d_gain, &self.d_mix, &self.d_bypass),
        ];
        let bands: [BandSettings; NUM_BANDS] = std::array::from_fn(|b| {
            let (thr, rat, atk, rls, kne, gain, mix, byp) = p[b];
            BandSettings {
                thresh_db: thr.value(),
                ratio: rat.value(),
                attack_ms: atk.value(),
                release_ms: rls.value(),
                knee_db: kne.value(),
                makeup_db: gain.value(),
                mix: mix.value() * 0.01,
                bypass: byp.value(),
                audible: !mute[b] && (!any_solo || solo[b]),
            }
        });
        Settings {
            bands,
            xover: [self.xover1.value(), self.xover2.value(), self.xover3.value()],
            in_db: self.in_gain.value(),
            out_db: self.out_gain.value(),
            mix: self.mix.value() * 0.01,
            lookahead_ms: self.lookahead.value(),
            bypass: self.bypass.value(),
        }
    }
}

pub struct Mbc4 {
    proc: Processor,
    work: Box<[[f32; CHUNK]; MAX_CH]>,
}

impl Default for Mbc4 {
    fn default() -> Self {
        Self { proc: Processor::default(), work: Box::new([[0.0; CHUNK]; MAX_CH]) }
    }
}

impl PluginLogic for Mbc4 {
    type Params = Mbc4Params;
    type DspState = Self;

    fn reset(state: &mut Self, params: &Mbc4Params, config: &AudioConfig) {
        params.shared.sample_rate.store(config.sample_rate as u32, std::sync::atomic::Ordering::Relaxed);
        state.proc.prepare(config.sample_rate, &params.settings(), &params.shared);
    }

    fn process(
        state: &mut Self,
        params: &Mbc4Params,
        buffer: &mut AudioBuffer,
        _events: &EventList,
        _context: &mut ProcessContext,
    ) -> ProcessStatus {
        let n = buffer.num_samples();
        let nch = buffer.num_input_channels().min(buffer.num_output_channels()).min(MAX_CH);
        for ch in nch..buffer.num_output_channels() {
            buffer.output(ch).fill(0.0);
        }
        if nch == 0 || n == 0 {
            return ProcessStatus::Normal;
        }

        let set = params.settings();
        let sh = &params.shared;
        let work = &mut *state.work;
        state.proc.begin_block(&set);
        let mut pos = 0;
        while pos < n {
            let len = (n - pos).min(CHUNK);
            for c in 0..nch {
                work[c][..len].copy_from_slice(&buffer.input(c)[pos..pos + len]);
            }
            state.proc.process(work, nch, len, &set, sh);
            for c in 0..nch {
                buffer.output(c)[pos..pos + len].copy_from_slice(&work[c][..len]);
            }
            pos += len;
        }
        state.proc.end_block(&set, sh);
        ProcessStatus::Normal
    }

    fn latency(state: &Self) -> u32 {
        state.proc.latency() as u32
    }

    fn editor(params: Arc<Mbc4Params>) -> Box<dyn Editor> {
        ui::editor(params)
    }
}

truce::plugin! {
    logic: Mbc4,
    params: Mbc4Params,
}

truce::enable_rt_paranoid!();

#[cfg(test)]
mod tests;
