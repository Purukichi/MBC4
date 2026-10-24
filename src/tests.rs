//! Offline checks: crossover flatness, compression, mix, gain match, lookahead, CPU,
//! plus truce's own conformance checks.

use super::*;
use crate::shared::Shared;
use std::f32::consts::TAU;

const SR: f64 = 48000.0;
const BLOCK: usize = 256;

fn db(g: f32) -> f32 {
    dsp::gain_to_db(g, -150.0)
}

fn rms_db(x: &[f32]) -> f32 {
    db((x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt())
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

/// defaults of the real parameter set, adjusted by `f`
fn settings(f: impl FnOnce(&mut Settings)) -> Settings {
    let mut s = Mbc4Params::new().settings();
    f(&mut s);
    s
}

fn prepared(set: &Settings, sh: &Shared) -> Processor {
    let mut p = Processor::default();
    p.prepare(SR, set, sh);
    p
}

/// stereo, identical channels, in host-sized blocks
fn run(p: &mut Processor, set: &Settings, sh: &Shared, x: &mut [f32]) {
    let mut r = x.to_vec();
    for (l, r) in x.chunks_mut(BLOCK).zip(r.chunks_mut(BLOCK)) {
        p.run(&mut [l, r], set, sh);
    }
}

fn sine(n: usize, hz: f32, amp: f32) -> Vec<f32> {
    (0..n).map(|i| amp * (TAU * hz * i as f32 / SR as f32).sin()).collect()
}

/// magnitude response of an impulse response, min / max dB over 20 Hz – 20 kHz
fn flatness(ir: &[f32]) -> (f32, f32) {
    let n = ir.len();
    let fft = realfft::RealFftPlanner::<f32>::new().plan_fft_forward(n);
    let mut input = ir.to_vec();
    let mut spec = fft.make_output_vec();
    fft.process(&mut input, &mut spec).unwrap();
    let (mut lo, mut hi) = (100.0f32, -100.0f32);
    for (i, z) in spec.iter().enumerate().take(n / 2).skip(1) {
        let f = i as f64 * SR / n as f64;
        if (20.0..=20000.0).contains(&f) {
            let d = db(z.norm());
            lo = lo.min(d);
            hi = hi.max(d);
        }
    }
    (lo, hi)
}

/// 1 s of a 400 Hz sine at −6 dBFS (inside band B): level change of the 2nd half in dB
fn sine_change(p: &mut Processor, set: &Settings, sh: &Shared) -> f32 {
    let n = 48000;
    let mut x = sine(n, 400.0, dsp::db_to_gain(-6.0));
    let in_db = rms_db(&x[n / 2..]);
    run(p, set, sh, &mut x);
    rms_db(&x[n / 2..]) - in_db
}

#[test]
fn crossover_sum_is_flat() {
    let set = settings(|s| s.bands.iter_mut().for_each(|b| b.ratio = 1.0));
    let sh = Shared::default();
    let mut p = prepared(&set, &sh);
    let mut x = vec![0.0f32; 1 << 15];
    x[0] = 1.0;
    run(&mut p, &set, &sh, &mut x);
    let (lo, hi) = flatness(&x);
    assert!(hi - lo < 0.05 && hi.abs() < 0.05, "min {lo:.3} dB, max {hi:.3} dB");
}

#[test]
fn compresses_a_sine_in_band_b() {
    let set = settings(|s| {
        s.bands[1].thresh_db = -30.0;
        s.bands[1].ratio = 4.0;
        s.bands[1].knee_db = 0.0;
    });
    let sh = Shared::default();
    let mut p = prepared(&set, &sh);
    let change = sine_change(&mut p, &set, &sh);
    // peak −6 dB, thr −30, 4:1 → 24 dB over → GR = 18 dB; neighbouring bands leak
    // a little uncompressed signal, so the sum reduces slightly less
    let meter = sh.gr_db(1);
    assert!((meter + 17.9).abs() < 0.5 && change < -15.0, "change {change:.2} dB, meter {meter:.2}");
}

#[test]
fn band_mix_zero_is_no_compression() {
    let set = settings(|s| {
        s.bands[1].thresh_db = -30.0;
        s.bands[1].ratio = 4.0;
        s.bands[1].mix = 0.0;
    });
    let sh = Shared::default();
    let mut p = prepared(&set, &sh);
    let change = sine_change(&mut p, &set, &sh);
    assert!(change.abs() < 0.1, "level change {change:.2} dB");
}

#[test]
fn gain_match_learns_the_average_reduction() {
    let mut set = settings(|s| {
        s.bands[1].thresh_db = -30.0;
        s.bands[1].ratio = 4.0;
        s.bands[1].knee_db = 0.0;
    });
    let sh = Shared::default();
    sh.sample_rate.store(SR as u32, std::sync::atomic::Ordering::Relaxed);
    let mut p = prepared(&set, &sh);
    sh.start_learning(1);
    sine_change(&mut p, &set, &sh);
    let avg = sh.learned_average_db(1);
    let secs = sh.learned_seconds(1);
    sh.stop_learning(1);
    assert!(secs > 0.9 && avg < -16.0 && avg > -19.0, "average {avg:.2} dB over {secs:.2} s");

    set.bands[1].makeup_db = -avg;
    let change = sine_change(&mut p, &set, &sh);
    assert!(change.abs() < 1.5, "with match applied the sine level is restored: change {change:.2} dB");
}

#[test]
fn defaults_leave_minus_6_dbfs_untouched() {
    let set = settings(|_| {});
    let sh = Shared::default();
    let mut p = prepared(&set, &sh);
    let change = sine_change(&mut p, &set, &sh);
    assert!(change.abs() < 0.1, "change {change:.2} dB");
}

#[test]
fn lookahead_reports_latency_and_stays_flat() {
    let set = settings(|s| {
        s.bands.iter_mut().for_each(|b| b.ratio = 1.0);
        s.lookahead_ms = 5.0;
    });
    let sh = Shared::default();
    let mut p = prepared(&set, &sh);
    let mut x = vec![0.0f32; 1 << 15];
    x[0] = 1.0;
    run(&mut p, &set, &sh, &mut x);
    let expected = (0.005 * SR).round() as usize;
    let first = x.iter().position(|v| v.abs() > 1.0e-6);
    let (lo, hi) = flatness(&x[expected..]);
    assert_eq!(p.latency(), expected);
    assert_eq!(first, Some(expected));
    assert!(hi - lo < 0.05, "flat within {:.3} dB", hi - lo);
}

#[test]
fn lookahead_catches_the_transient() {
    let overshoot = |look_ms: f32| {
        // every band compresses, so the onset cannot leak through an idle band
        let set = settings(|s| {
            for b in &mut s.bands {
                b.thresh_db = -30.0;
                b.ratio = 20.0;
                b.knee_db = 0.0;
                b.attack_ms = 1.0;
            }
            s.lookahead_ms = look_ms;
        });
        let sh = Shared::default();
        let mut p = prepared(&set, &sh);
        // 0.5 s silence, then a −6 dBFS 400 Hz burst with a 1 ms fade-in
        let n = 48000;
        let amp = dsp::db_to_gain(-6.0);
        let mut x: Vec<f32> = (0..n)
            .map(|i| {
                if i < n / 2 {
                    return 0.0;
                }
                let k = (i - n / 2) as f32;
                (k / 48.0).min(1.0) * amp * (TAU * 400.0 * k / SR as f32).sin()
            })
            .collect();
        run(&mut p, &set, &sh, &mut x);
        let lat = p.latency();
        let onset = peak(&x[n / 2 + lat..n / 2 + lat + 480]);
        let steady = peak(&x[n - 4800..]);
        db(onset) - db(steady)
    };
    let (without, with) = (overshoot(0.0), overshoot(5.0));
    // what remains with lookahead is the crossover's own onset ringing
    assert!(with < 3.0 && with < without - 4.0, "{without:.1} dB without lookahead, {with:.1} dB with 5 ms");
}

#[test]
fn host_bypass_is_the_delayed_input() {
    let mut set = settings(|s| {
        s.lookahead_ms = 2.0;
        s.bands[1].thresh_db = -40.0;
    });
    let sh = Shared::default();
    let mut p = prepared(&set, &sh);
    let mut warm = sine(4800, 400.0, 0.5);
    run(&mut p, &set, &sh, &mut warm);
    set.bypass = true;
    let input = sine(9600, 400.0, 0.5);
    let mut x = input.clone();
    run(&mut p, &set, &sh, &mut x);
    let lat = p.latency();
    assert_eq!(lat, 96);
    assert!(x[lat..].iter().zip(&input).all(|(a, b)| a == b), "bypass must be the input delayed by {lat} samples");
}

#[cfg(not(debug_assertions))]
#[test]
fn cpu_under_3_percent_of_a_core() {
    let set = settings(|s| {
        s.lookahead_ms = 5.0;
        s.bands.iter_mut().for_each(|b| b.thresh_db = -30.0);
    });
    let sh = Shared::default();
    let mut p = prepared(&set, &sh);
    let seconds = 20;
    let mut seed = 1u32;
    let mut noise = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed as f32 / u32::MAX as f32) * 0.5 - 0.25
    };
    let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    let t0 = std::time::Instant::now();
    for _ in 0..(SR as usize * seconds / BLOCK) {
        for i in 0..BLOCK {
            l[i] = noise();
            r[i] = noise();
        }
        p.run(&mut [&mut l, &mut r], &set, &sh);
    }
    let pct = 100.0 * t0.elapsed().as_secs_f64() / seconds as f64;
    println!("CPU: {pct:.3} % of one core (stereo 48 kHz)");
    assert!(pct < 3.0, "{pct:.3} %");
}

// ── truce conformance

#[test]
fn info_is_valid() {
    truce_test::assert_valid_info::<Plugin>();
}

#[test]
fn state_round_trips() {
    truce_test::assert_state_round_trip::<Plugin>();
}

#[test]
fn param_defaults_match() {
    truce_test::assert_param_defaults_match::<Plugin>();
}

#[test]
fn no_duplicate_param_ids() {
    truce_test::assert_no_duplicate_param_ids::<Plugin>();
}

#[test]
fn has_editor() {
    truce_test::assert_has_editor::<Plugin>();
}

#[test]
fn reports_lookahead_as_latency() {
    use std::time::Duration;
    use truce_test::{InputSource, driver};
    let result = driver!(Plugin)
        .duration(Duration::from_millis(100))
        .input(InputSource::Constant(0.25))
        .script(|s| {
            s.set_param(Mbc4ParamsParamId::Lookahead, 0.5); // 5 ms
        })
        .run();
    assert!(!result.output[0].is_empty());
}

#[test]
fn process_is_allocation_free() {
    use std::time::Duration;
    use truce_test::{InputSource, assert_no_audio_alloc, driver};
    assert_no_audio_alloc(|| {
        driver!(Plugin)
            .duration(Duration::from_millis(40))
            .input(InputSource::Constant(0.25))
            .script(|s| {
                s.set_param(Mbc4ParamsParamId::BThresh, 0.3);
                s.set_param(Mbc4ParamsParamId::Lookahead, 0.4);
                s.wait_ms(15);
            })
            .run()
    });
}
