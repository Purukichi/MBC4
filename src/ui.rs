//! MBC4 editor: one painted surface on egui, PC1 style (`mockup/MBC4_PC1.html`).
//!
//! Everything is laid out in design units (1080 × 520 folded, 1210 × 520 with the INPUT / OUTPUT panel, DESIGN_CONCEPT.md) and scaled
//! uniformly, 75 – 125 %.
//! Cells are snapped to whole device pixels, so every cell and gap has one
//! size at any zoom.
//!
//! 01 INPUT | 02 SPLIT over 03 – 06 BAND A – D | 07 OUTPUT

use crate::dsp::NUM_BANDS;
use crate::shared::{RING, Ring, Shared};
use crate::{Mbc4Params, Mbc4ParamsParamId as P, hz_from_text, hz_short, ms_text, number_text};
use egui::{Align2, Color32, FontFamily, FontId, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};
use realfft::{RealFftPlanner, RealToComplex};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::time::{Instant, SystemTime};
use truce::prelude::*;
use truce_core::editor::PluginContext;
use truce_egui::{EditorUi, EguiEditor};
use truce_params::ParamRange;

type Ctx = PluginContext<Mbc4Params>;

// ───────────────────────────────── tokens

/// width of the folded layout; the INPUT / OUTPUT panel adds IO_W on the right
const W: f32 = COL_R + 20.0;
const IO_W: f32 = 130.0;
const H: f32 = FRAME_B + 40.0;
/// "100 %": the design units as they are (the presets and the corner drag are relative to it)
const DEFAULT_SCALE: f32 = 1.0;
/// 75 – 125 % (the corner drag and the size MBC4 asks the host for)
const SCALE_RANGE: (f32, f32) = (0.75 * DEFAULT_SCALE, 1.25 * DEFAULT_SCALE);
/// footer presets, the last one ending at the frame's right edge
const ZOOMS: [(f32, &str); 3] = [(0.75 * DEFAULT_SCALE, "75"), (DEFAULT_SCALE, "100"), (1.25 * DEFAULT_SCALE, "125")];
/// Hard floor given to the host: a host that keeps a smaller window gets the
/// whole UI scaled down to fit instead of a clipped one.
const MIN_FIT_SCALE: f32 = 0.5;

fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}
/// `b` laid over `a` at opacity `t`, as an opaque colour
fn lerp_col(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// One theme's colours (DESIGN_CONCEPT.md §2). Every draw looks them up through
/// `pal()`, so a theme switch repaints everything on the next frame.
struct Pal {
    /// the one background
    cell: u32,
    rule: u32,
    /// text, fader caps, reference lines, bubble ground
    ink: u32,
    /// secondary text, scale numbers
    ink2: u32,
    /// notes, footer buttons, off toggles
    ink3: u32,
    /// unlit cells, pill grooves
    off: u32,
    /// lit cells (fader amounts)
    lit: u32,
    /// data dots (spectrum, meters)
    kept: u32,
    /// selected segment, entry field
    plot: u32,
    /// signal orange
    acc: u32,
    /// input spectrum behind the output (not heard)
    ghost: u32,
    // disabled (bypassed band)
    d_lit: u32,
    d_knob: u32,
    d_text: u32,
    /// text on an `ink` ground (bubbles)
    on_ink: u32,
    tip_bg: (u32, u8),
    tip_text: u32,
}
impl Pal {
    const LIGHT: Pal = Pal {
        cell: 0xEDEEEA,
        rule: 0xB8B9B4,
        ink: 0x0D0E0C,
        ink2: 0x454642,
        ink3: 0x7C7D78,
        off: 0xDADBD6,
        lit: 0xA9AAA5,
        kept: 0x969792,
        plot: 0xF9F9F6,
        acc: 0xFF4D0F,
        ghost: 0xCFD0CB,
        d_lit: 0xC6C7C2,
        d_knob: 0xA0A19C,
        d_text: 0xA7A8A3,
        on_ink: 0xF9F9F6,
        tip_bg: (0x0D0E0C, 245),
        tip_text: 0xF9F9F6,
    };
    const DARK: Pal = Pal {
        cell: 0x171816,
        rule: 0x3D3E3B,
        ink: 0xE9EAE6,
        ink2: 0xB3B4AF,
        ink3: 0x83847F,
        off: 0x262724,
        lit: 0x50514D,
        kept: 0x60615C,
        plot: 0x30312E,
        acc: 0xFF4D0F,
        ghost: 0x2E2F2C,
        d_lit: 0x343532,
        d_knob: 0x4F504C,
        d_text: 0x4C4D49,
        on_ink: 0x171816,
        tip_bg: (0x2A2B28, 247),
        tip_text: 0xE9EAE6,
    };
}

// ───────────────────────────────── theme (one setting for every PC1-style plugin)

static DARK: AtomicBool = AtomicBool::new(false);

fn pal() -> &'static Pal {
    if DARK.load(Relaxed) { &Pal::DARK } else { &Pal::LIGHT }
}

/// `<config dir>/PC1-style/settings.cfg`: `key = value` lines, shared by the whole family.
fn settings_path() -> Option<PathBuf> {
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let base = if cfg!(windows) {
        env("APPDATA")
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Application Support"))
    } else {
        env("XDG_CONFIG_HOME").or_else(|| env("HOME").map(|h| h.join(".config")))
    };
    base.map(|b| b.join("PC1-style").join("settings.cfg"))
}

/// No file, or one that can't be read: light.
fn load_theme() {
    let text = settings_path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let dark = text.lines().filter_map(|l| l.split_once('=')).any(|(k, v)| k.trim() == "theme" && v.trim() == "dark");
    DARK.store(dark, Relaxed);
}

/// Rewrites only the `theme` line, keeping whatever else other plugins keep there.
fn save_theme(dark: bool) {
    let Some(path) = settings_path() else { return };
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> =
        old.lines().filter(|l| l.split_once('=').is_none_or(|(k, _)| k.trim() != "theme")).map(str::to_owned).collect();
    lines.push(format!("theme = {}", if dark { "dark" } else { "light" }));
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, lines.join("\n") + "\n");
}

fn settings_mtime() -> Option<SystemTime> {
    settings_path().and_then(|p| std::fs::metadata(p).ok()).and_then(|m| m.modified().ok())
}

/// Chivo Mono Regular / Light (SIL OFL 1.1, `resources/fonts/OFL.txt`)
const CHIVO: &[u8] = include_bytes!("../resources/fonts/ChivoMono-Regular.ttf");
const CHIVO_LIGHT: &[u8] = include_bytes!("../resources/fonts/ChivoMono-Light.ttf");

/// The four type roles. Sizes are em heights in design units.
#[derive(Clone, Copy, PartialEq)]
enum Role {
    /// labels, section names, buttons: Regular 8, tracking 0.8
    Label,
    /// values: Light 9
    Value,
    /// notes (version, section numbers): Light 7.5, tracking 0.6
    Note,
    /// tick numbers: Light 7.5
    Tick,
    /// the number riding on a band's GR bar: Light 6
    Tiny,
}
use Role::{Label as LABEL, Note as NOTE, Tick as TICK, Tiny as TINY, Value as VAL};

impl Role {
    fn size(self) -> f32 {
        match self {
            Role::Label => 8.0,
            Role::Value => 9.0,
            Role::Note | Role::Tick => 7.5,
            Role::Tiny => 6.0,
        }
    }
    fn tracking(self) -> f32 {
        match self {
            Role::Label => 0.8,
            Role::Note => 0.6,
            Role::Value | Role::Tick | Role::Tiny => 0.0,
        }
    }
    fn light(self) -> bool {
        self != Role::Label
    }
}

// ───────────────────────────────── layout grid (10 px module; cells 3 × 3 on a 4 px pitch)

const FRAME_L: f32 = 20.0;
const FRAME_T: f32 = 20.0;
/// right below the last slider row (= the fader value row)
const FRAME_B: f32 = FADER_VALUE_Y + 16.0;
/// footer: every text and icon centre
const FOOT_Y: f32 = FRAME_B + 11.0;
/// input | main | output, both side columns 95 px (the 75 px logo with 10 on each side)
const COL_L: f32 = FRAME_L;
const COL_R: f32 = COL_L + STRIP_W * NUM_BANDS as f32;
/// graph | band strips
const ROW_B: f32 = 320.0;
const STRIP_W: f32 = 260.0;
const PITCH: f32 = 4.0;
const CELL: f32 = 3.0;
// graph plot: +6 .. −72 dB
const PLOT_L: f32 = COL_L + 10.0;
const PLOT_R: f32 = COL_R - 50.0;
const PLOT_T: f32 = 80.0;
const PLOT_B: f32 = 280.0;
const MAX_DB: f32 = 6.0;
const MIN_DB: f32 = -72.0;
/// meter peak readouts / frequency labels (centre line): 12 below the plot
const READOUT_TOP: f32 = PLOT_B + 12.0;
// vertical faders: label row, 23 cells, value row level with the last slider row
const FADER_TOP: f32 = ROW_B + 20.0;
const FADER_LABEL_Y: f32 = FADER_TOP + 7.0;
const FADER_CELLS_Y: f32 = FADER_TOP + 22.0;
const FADER_VALUE_Y: f32 = FADER_CELLS_Y + 102.0;
const FADER_H: f32 = FADER_VALUE_Y + 7.0 - FADER_TOP;
const FADER_ROWS: usize = 23;
/// clickable width (they paint 44 wide)
const FADER_GRAB: f32 = 28.0;
// horizontal sliders (244 × 14, 36 cells): centre of the first row, level with the first fader cells
const SLIDER_Y0: f32 = FADER_CELLS_Y + 2.0;
const SLIDER_PITCH: f32 = 20.0;
const SLIDER_CELLS: usize = 36;
const SLIDER_W: f32 = 100.0 + 4.0 * SLIDER_CELLS as f32;
// band header x offset: letter, then the range
// GR on the graph: a line under the left end of the threshold line, 2 px a dB
const GR_LEN: f32 = 48.0;
const GR_BELOW: f32 = 6.0;
/// three times the threshold line
const GR_LINE: f32 = 1.5;
const HEADER_Y: f32 = ROW_B + 15.0;
const RANGE_X: f32 = 22.0;

const MIN_HZ: f64 = 20.0;
const MAX_HZ: f64 = 20000.0;

fn strip_x(b: usize) -> f32 {
    COL_L + STRIP_W * b as f32
}
fn freq_to_x(f: f64) -> f32 {
    PLOT_L + (PLOT_R - PLOT_L) * ((f / MIN_HZ).ln() / (MAX_HZ / MIN_HZ).ln()) as f32
}
fn x_to_freq(x: f32) -> f64 {
    MIN_HZ * (MAX_HZ / MIN_HZ).powf(f64::from((x - PLOT_L) / (PLOT_R - PLOT_L)))
}
fn db_to_y(db: f32) -> f32 {
    PLOT_T + (MAX_DB - db) / (MAX_DB - MIN_DB) * (PLOT_B - PLOT_T)
}
fn y_to_db(y: f32) -> f32 {
    MAX_DB - (y - PLOT_T) / (PLOT_B - PLOT_T) * (MAX_DB - MIN_DB)
}
fn rows_for(db: f32, rows: usize) -> usize {
    let per_row = (MAX_DB - MIN_DB) / rows as f32;
    ((db - MIN_DB) / per_row + 1.0e-4).floor().clamp(0.0, rows as f32) as usize
}
fn in_r(r: [f32; 4], p: Pos2) -> bool {
    p.x >= r[0] && p.x < r[0] + r[2] && p.y >= r[1] && p.y < r[1] + r[3]
}
fn hz_text(f: f64) -> String {
    if f >= 1000.0 { format!("{}Hz", hz_short(f)) } else { format!("{:.0} Hz", f.round()) }
}
fn round_significant(f: f64, digits: i32) -> f64 {
    let e = 10f64.powf(f.log10().floor() - f64::from(digits - 1));
    (f / e).round() * e
}
const LETTERS: [&str; 4] = ["A", "B", "C", "D"];
const DASH: &str = "\u{2013}";

// ───────────────────────────────── values

#[derive(Clone, Copy, PartialEq)]
enum Format {
    Db,
    Ms,
    Percent,
    Ratio,
}

fn format_value(v: f64, f: Format) -> String {
    match f {
        Format::Db => format!("{} dB", number_text(v)),
        Format::Ms => format!("{} ms", ms_text(v)),
        Format::Percent => format!("{:.0} %", v.round()),
        Format::Ratio => {
            if v >= 19.95 {
                "20:1".into()
            } else {
                format!("{}:1", number_text(v))
            }
        }
    }
}

/// How a fader moves: plain drags and the wheel stop on fixed steps (or on a list of
/// values), Shift or Ctrl moves finely.
#[derive(Clone, Copy)]
struct Steps {
    step: f64,
    fine: f64,
    /// when set, plain moves stop only on these values
    list: &'static [f64],
}

const DB_STEPS: Steps = Steps { step: 1.0, fine: 0.1, list: &[] };
const RATIO_STEPS: Steps = Steps { step: 1.0, fine: 0.1, list: &[1.0, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0, 10.0, 20.0] };
const ATK_STEPS: Steps = Steps {
    step: 1.0,
    fine: 0.1,
    list: &[0.1, 0.3, 1.0, 2.0, 3.0, 5.0, 8.0, 10.0, 15.0, 20.0, 25.0, 30.0, 50.0, 80.0, 100.0, 150.0, 200.0, 300.0],
};
const RLS_STEPS: Steps = Steps {
    step: 1.0,
    fine: 1.0,
    list: &[5.0, 10.0, 20.0, 30.0, 50.0, 80.0, 120.0, 180.0, 250.0, 350.0, 500.0, 750.0, 1000.0, 1500.0, 2000.0, 3000.0],
};
const MIX_STEPS: Steps = Steps { step: 5.0, fine: 1.0, list: &[] };
const LOOK_STEPS: Steps = Steps { step: 0.5, fine: 0.1, list: &[] };

#[derive(Clone, Copy, PartialEq)]
enum Tip {
    Gain,
    Ratio,
    Knee,
    Attack,
    Release,
    BandMix,
    Solo,
    Mute,
    Bypass,
    Match,
    Input,
    Look,
    Mix,
    Output,
    Reset,
    Theme,
    Zoom,
    Io,
}

impl Tip {
    fn text(self) -> (&'static str, &'static str) {
        match self {
            Tip::Gain => ("Gain", "バンドの出力ゲイン（メイクアップ）。"),
            Tip::Ratio => ("Ratio", "圧縮の比率。"),
            Tip::Knee => ("Knee", "スレッショルド付近を滑らかにする幅。"),
            Tip::Attack => ("Attack", "圧縮がかかり始めるまでの速さ。"),
            Tip::Release => ("Release", "圧縮が戻るまでの速さ。"),
            Tip::BandMix => ("Mix", "バンド内のドライ／ウェットの割合。"),
            Tip::Solo => ("Solo", "このバンドだけを聴きます。"),
            Tip::Mute => ("Mute", "このバンドを消音します。"),
            Tip::Bypass => ("Bypass", "このバンドの圧縮を止めます。"),
            Tip::Match => ("Gain match", "3 秒間の平均ゲインリダクションを測って、ゲインを 1 回だけ合わせます。もう一度押すと取り消します。"),
            Tip::Input => ("Input", "入力ゲイン。"),
            Tip::Look => ("Lookahead", "先読み時間。レイテンシとしてホストに報告します。"),
            Tip::Mix => ("Mix", "全体のドライ／ウェットの割合。"),
            Tip::Output => ("Output", "出力ゲイン。"),
            Tip::Reset => ("Reset", "すべてのつまみとクロスオーバーを初期値に戻します。Ctrl+Z で元に戻せます。"),
            Tip::Theme => ("Theme", "ライト / ダークを切り替えます。すべての PC1 系プラグインで共通の設定です。"),
            Tip::Zoom => ("UI size", "エディターの大きさ。右下の角をドラッグすると 75 – 125 % の間で細かく変えられます。"),
            Tip::Io => ("I/O", "INPUT / OUTPUT（入出力ゲイン、メーター、lookahead、全体の mix）を右側に開きます。もう一度押すと格納します。"),
        }
    }
}

// band parameter slots
const THR: usize = 0;
const RAT: usize = 1;
const ATK: usize = 2;
const RLS: usize = 3;
const KNE: usize = 4;
const GAIN: usize = 5;
const MIX: usize = 6;
const SOLO: usize = 7;
const BYP: usize = 9;

#[rustfmt::skip]
const BAND_IDS: [[P; 10]; NUM_BANDS] = [
    [P::AThresh, P::ARatio, P::AAttack, P::ARelease, P::AKnee, P::AGain, P::AMix, P::ASolo, P::AMute, P::ABypass],
    [P::BThresh, P::BRatio, P::BAttack, P::BRelease, P::BKnee, P::BGain, P::BMix, P::BSolo, P::BMute, P::BBypass],
    [P::CThresh, P::CRatio, P::CAttack, P::CRelease, P::CKnee, P::CGain, P::CMix, P::CSolo, P::CMute, P::CBypass],
    [P::DThresh, P::DRatio, P::DAttack, P::DRelease, P::DKnee, P::DGain, P::DMix, P::DSolo, P::DMute, P::DBypass],
];
const XO_IDS: [P; 3] = [P::Xover1, P::Xover2, P::Xover3];

fn bid(b: usize, slot: usize) -> u32 {
    BAND_IDS[b][slot].into()
}

// ───────────────────────────────── controls

/// Fader drawn with cells: the cells from the base value to the current value are lit,
/// the value's own cell (the whole row) is the knob, in `ink`; nothing else is drawn.
#[derive(Clone, Copy)]
struct Fader {
    id: u32,
    vertical: bool,
    /// vertical: centre x; horizontal: left edge
    x: f32,
    /// vertical: top (FADER_TOP); horizontal: centre line
    y: f32,
    band: Option<usize>,
    label: &'static str,
    steps: Steps,
    format: Format,
    tip: Tip,
    /// the knob sits on the input / output meter (its rows), the value under the readout
    on_meter: bool,
}

impl Fader {
    fn cells(&self) -> usize {
        if self.vertical { FADER_ROWS } else { SLIDER_CELLS }
    }
    fn rect(&self) -> [f32; 4] {
        if self.on_meter {
            [self.x - FADER_GRAB / 2.0, PLOT_T, FADER_GRAB, PLOT_B - PLOT_T]
        } else if self.vertical { [self.x - FADER_GRAB / 2.0, self.y, FADER_GRAB, FADER_H] } else { [self.x, self.y - 7.0, SLIDER_W, 14.0] }
    }
    fn value_rect(&self) -> [f32; 4] {
        if self.on_meter {
            [self.x - 22.0, METER_VALUE_Y - 7.0, 44.0, 14.0]
        } else if self.vertical { [self.x - 22.0, FADER_VALUE_Y - 7.0, 44.0, 14.0] } else { [self.x + SLIDER_W - 54.0, self.y - 7.0, 50.0, 14.0] }
    }
}

fn faders() -> Vec<Fader> {
    let mut v = Vec::new();
    for b in 0..NUM_BANDS {
        let x0 = strip_x(b);
        // the threshold lives on the graph; everything else is a horizontal slider
        for (j, (slot, label, steps, format, tip)) in [
            (RAT, "RAT", RATIO_STEPS, Format::Ratio, Tip::Ratio),
            (KNE, "KNE", DB_STEPS, Format::Db, Tip::Knee),
            (ATK, "ATK", ATK_STEPS, Format::Ms, Tip::Attack),
            (RLS, "RLS", RLS_STEPS, Format::Ms, Tip::Release),
            (GAIN, "GAIN", DB_STEPS, Format::Db, Tip::Gain),
            (MIX, "MIX", MIX_STEPS, Format::Percent, Tip::BandMix),
        ]
        .into_iter()
        .enumerate()
        {
            let y = SLIDER_Y0 + SLIDER_PITCH * j as f32;
            v.push(Fader { id: bid(b, slot), vertical: false, x: x0 + 6.0, y, band: Some(b), label, steps, format, tip, on_meter: false });
        }
    }
    // input / output gain: knobs on the meters
    for (id, cx, label, tip) in [(P::InGain, IN_METER_X, "IN", Tip::Input), (P::OutGain, OUT_METER_X, "OUT", Tip::Output)] {
        v.push(Fader { id: id.into(), vertical: true, x: cx, y: PLOT_T, band: None, label, steps: DB_STEPS, format: Format::Db, tip, on_meter: true });
    }
    for (id, cx, label, steps, format, tip) in [
        (P::Lookahead, OUT_COL - 18.0, "LOOK", LOOK_STEPS, Format::Ms, Tip::Look),
        (P::Mix, OUT_COL + 18.0, "MIX", MIX_STEPS, Format::Percent, Tip::Mix),
    ] {
        v.push(Fader { id: id.into(), vertical: true, x: cx, y: FADER_TOP, band: None, label, steps, format, tip, on_meter: false });
    }
    v
}

/// the gain value under a meter's dot readout
const METER_VALUE_Y: f32 = READOUT_TOP + 14.0;
/// the INPUT / OUTPUT panel: two 65 px columns right of the bands (one LOOK / MIX block below)
const OUT_COL: f32 = COL_R + IO_W / 2.0;
const IN_METER_X: f32 = COL_R + IO_W / 4.0;
const OUT_METER_X: f32 = OUT_COL + IO_W / 4.0;
/// I/O: the letter button at the right end of the top row
const IO_RECT: [f32; 4] = [COL_R - 10.0 - 24.0, FRAME_T + 7.0, 24.0, 16.0];
/// the editor's design width for the panel state
fn width_of(open: bool) -> f32 {
    if open { W + IO_W } else { W }
}
fn frame_r(open: bool) -> f32 {
    width_of(open) - 20.0
}

/// S / M / B letter buttons (12 × 16 each, no gaps), the last one ending 10 inside the strip
fn toggle_rect(b: usize, j: usize) -> [f32; 4] {
    [strip_x(b) + STRIP_W - 10.0 - 12.0 * (3 - j) as f32, ROW_B + 7.0, 12.0, 16.0]
}
/// gain match: in the header, 6 left of S / M / B
fn match_rect(b: usize) -> [f32; 4] {
    [toggle_rect(b, 0)[0] - 6.0 - 42.0, ROW_B + 7.0, 42.0, 16.0]
}

#[derive(Clone, Copy, PartialEq)]
enum Hit {
    Fader(usize),
    Toggle(usize, usize),
    Match(usize),
    Xo(usize),
    Thresh(usize),
    Plot,
    /// band header: low / high edge frequency text
    BandLo(usize),
    BandHi(usize),
    /// footer: 0 = reset, 1 = theme
    Icon(usize),
    /// opens / folds the INPUT / OUTPUT panel
    Io,
    Zoom(usize),
    Resize,
}

#[derive(Clone, Copy, PartialEq)]
enum EditTarget {
    Fader(usize),
    Xo(usize),
}

// ───────────────────────────────── painter in design units

/// Cells on a whole-device-pixel pitch, so every cell and every gap has the same size
/// at any UI scale. Device-pixel fields; `k` = device pixels per design unit.
#[derive(Clone, Copy)]
struct Lat {
    p: f32,
    gap: f32,
    c: f32,
    x0: f32,
    y0: f32,
    k: f32,
}

impl Lat {
    /// cells that fit a length
    fn fit(&self, len: f32) -> usize {
        (((len * self.k).round() + self.gap) / self.p).floor().max(1.0) as usize
    }
    /// left edge of column i (design units)
    fn x(&self, i: usize) -> f32 {
        (self.x0 + i as f32 * self.p) / self.k
    }
    /// top edge of row j
    fn y(&self, j: usize) -> f32 {
        (self.y0 + j as f32 * self.p) / self.k
    }
    fn cell(&self) -> f32 {
        self.c / self.k
    }
    fn gap(&self) -> f32 {
        self.gap / self.k
    }
    /// length of n cells
    fn ext(&self, n: usize) -> f32 {
        (n as f32 * self.p - self.gap) / self.k
    }
}

struct Pen<'a> {
    p: &'a egui::Painter,
    o: Pos2,
    s: f32,
    ppp: f32,
    k: f32,
    /// the Light family is registered on this context
    light: bool,
}

impl Pen<'_> {
    /// device-pixel coordinates → points
    fn px(&self, x: f32, y: f32) -> Pos2 {
        pos2(self.o.x + x / self.ppp, self.o.y + y / self.ppp)
    }
    fn at(&self, x: f32, y: f32) -> Pos2 {
        pos2(self.o.x + x * self.s, self.o.y + y * self.s)
    }
    fn lat(&self, x: f32, y: f32, pitch: f32, cell: f32) -> Lat {
        let gap = ((pitch - cell) * self.k).round().max(1.0);
        let p = (pitch * self.k).round().max(gap + 1.0);
        Lat { p, gap, c: p - gap, x0: (x * self.k).round(), y0: (y * self.k).round(), k: self.k }
    }
    fn cell(&self, l: &Lat, i: usize, j: usize, col: Color32) {
        self.span(l, i, j, 1, 1, col);
    }
    /// cells [i, i+ni) × [j, j+nj), including the gaps between them
    fn span(&self, l: &Lat, i: usize, j: usize, ni: usize, nj: usize, col: Color32) {
        let x = l.x0 + i as f32 * l.p;
        let y = l.y0 + j as f32 * l.p;
        let r = Rect::from_min_max(self.px(x, y), self.px(x + ni as f32 * l.p - l.gap, y + nj as f32 * l.p - l.gap));
        self.p.rect_filled(r, 0.0, col);
    }
    fn fill(&self, x: f32, y: f32, w: f32, h: f32, col: Color32) {
        self.p.rect_filled(Rect::from_min_max(self.at(x, y), self.at(x + w, y + h)), 0.0, col);
    }
    /// snapped to device pixels, at least one device pixel
    fn rect(&self, x: f32, y: f32, w: f32, h: f32, col: Color32) {
        let (x0, y0) = ((x * self.k).round(), (y * self.k).round());
        let x1 = ((x + w) * self.k).round().max(x0 + 1.0);
        let y1 = ((y + h) * self.k).round().max(y0 + 1.0);
        self.p.rect_filled(Rect::from_min_max(self.px(x0, y0), self.px(x1, y1)), 0.0, col);
    }
    fn dev(&self, t: f32) -> f32 {
        (t * self.k).round().max(1.0)
    }
    /// reference lines: never under one device pixel
    fn line_h(&self, x1: f32, x2: f32, y: f32, col: Color32, t: f32) {
        let d = self.dev(t);
        let y0 = (y * self.k - d / 2.0).round();
        let (a, b) = ((x1 * self.k).round(), (x2 * self.k).round());
        self.p.rect_filled(Rect::from_min_max(self.px(a, y0), self.px(b, y0 + d)), 0.0, col);
    }
    fn line_v(&self, x: f32, y1: f32, y2: f32, col: Color32, t: f32) {
        let d = self.dev(t);
        let x0 = (x * self.k - d / 2.0).round();
        let (a, b) = ((y1 * self.k).round(), (y2 * self.k).round());
        self.p.rect_filled(Rect::from_min_max(self.px(x0, a), self.px(x0 + d, b)), 0.0, col);
    }
    /// (family, tracking) for a type role
    fn style(&self, role: Role) -> (FontFamily, f32) {
        let family = if role.light() && self.light { FontFamily::Name("light".into()) } else { FontFamily::Monospace };
        (family, role.tracking())
    }
    fn galley(&self, s: &str, role: Role, col: Color32) -> Arc<egui::Galley> {
        let (family, ls) = self.style(role);
        let fmt = egui::TextFormat {
            font_id: FontId::new(role.size() * self.s, family),
            color: col,
            extra_letter_spacing: ls * self.s,
            ..Default::default()
        };
        self.p.layout_job(egui::text::LayoutJob::single_section(s.to_owned(), fmt))
    }
    /// y is the text's centre line
    fn text(&self, s: &str, x: f32, y: f32, role: Role, col: Color32, align: Align2) {
        let g = self.galley(s, role, col);
        let (_, ls) = self.style(role);
        // tracking also trails the last glyph: centre / right-align on the ink
        let w = g.size().x - if matches!(align.x(), egui::Align::Min) { 0.0 } else { ls * self.s };
        let r = align.anchor_size(self.at(x, y), vec2(w, g.size().y));
        self.p.galley(r.min, g, col);
    }
    fn text_w(&self, s: &str, role: Role) -> f32 {
        let (_, ls) = self.style(role);
        (self.galley(s, role, Color32::WHITE).size().x - ls * self.s) / self.s
    }
    fn pill(&self, x: f32, y: f32, w: f32, h: f32, fill: Color32, stroke: Option<Color32>) {
        let r = Rect::from_min_max(self.at(x, y), self.at(x + w, y + h));
        let rad = (w.min(h) / 2.0 * self.s) as u8;
        self.p.rect_filled(r, rad, fill);
        if let Some(c) = stroke {
            self.p.rect_stroke(r, rad, Stroke::new(1.0 / self.ppp, c), StrokeKind::Inside);
        }
    }
    fn bubble(&self, s: &str, cx: f32, cy: f32, min_x: f32, max_x: f32) {
        let w = self.text_w(s, VAL) + 10.0;
        let h = 14.0;
        let x = (cx - w / 2.0).min((max_x - w).max(min_x)).max(min_x);
        let r = Rect::from_min_max(self.at(x, cy - h / 2.0), self.at(x + w, cy + h / 2.0));
        self.p.rect_filled(r, (3.0 * self.s) as u8, hex(pal().ink));
        self.text(s, x + w / 2.0, cy, VAL, hex(pal().on_ink), Align2::CENTER_CENTER);
    }
    /// section name only (no number)
    fn section_header(&self, name: &str, x: f32, y: f32, col: Color32) {
        self.text(name, x, y, LABEL, col, Align2::LEFT_CENTER);
    }
    /// Line icon in a 7.5 box (the note text's size) centred on (cx, cy); strokes 1/8 of the box.
    fn icon(&self, which: usize, cx: f32, cy: f32, col: Color32) {
        const BOX: f32 = 7.5;
        let t = (BOX / 8.0 * self.s).max(1.0 / self.ppp);
        let stroke = Stroke::new(t, col);
        let ts = t / self.s;
        if which == 0 {
            // reset: a full-size circle open between 1 and 2 o'clock, a filled arrow head on the 2 o'clock end
            let r = BOX / 2.0 - ts / 2.0;
            let deg = std::f32::consts::PI / 180.0;
            let (a0, a1) = (-30.0 * deg, 300.0 * deg);
            let pt = |a: f32, rr: f32| self.at(cx + rr * a.cos(), cy + rr * a.sin());
            let arc: Vec<Pos2> = (0..=32).map(|i| pt(a0 + (a1 - a0) * i as f32 / 32.0, r)).collect();
            self.p.add(egui::Shape::line(arc, stroke));
            // pointing along the circle, into the opening
            let (ex, ey) = (cx + r * a0.cos(), cy + r * a0.sin());
            let (tx, ty) = (a0.sin(), -a0.cos());
            let (len, half) = (2.4, 1.7);
            let tri = vec![
                self.at(ex + tx * len, ey + ty * len),
                self.at(ex - ty * half, ey + tx * half),
                self.at(ex + ty * half, ey - tx * half),
            ];
            self.p.add(egui::Shape::convex_polygon(tri, col, Stroke::NONE));
        } else {
            // theme: a full-size square, left half filled
            let r = Rect::from_center_size(self.at(cx, cy), vec2(BOX, BOX) * self.s);
            self.p.rect_stroke(r, 0.0, stroke, StrokeKind::Inside);
            self.p.rect_filled(Rect::from_min_max(r.min, pos2(r.center().x, r.max.y)), 0.0, col);
        }
    }
}

/// UI size buttons: ink width + 6, the last one ending at the frame's right edge (`fr`)
fn zoom_rects(pen: &Pen<'_>, fr: f32) -> [[f32; 4]; 3] {
    let mut out = [[0.0; 4]; 3];
    let mut x = fr;
    for i in (0..ZOOMS.len()).rev() {
        let w = pen.text_w(ZOOMS[i].1, NOTE) + 6.0;
        x -= w;
        out[i] = [x, FOOT_Y - 7.0, w, 14.0];
    }
    out
}
/// footer icons: theme 10 left of the size buttons, reset right next to it (7.5 + 6 wide)
fn icon_rect(pen: &Pen<'_>, fr: f32, i: usize) -> [f32; 4] {
    const IW: f32 = 7.5 + 6.0;
    let theme_x = zoom_rects(pen, fr)[0][0] - 10.0 - IW;
    [if i == 1 { theme_x } else { theme_x - IW }, FOOT_Y - 7.0, IW, 14.0]
}

/// 4 × 5 dot logo, the last letter in the accent
fn logo(pen: &Pen<'_>, x: f32, y: f32) {
    const LETTERS: [[&str; 5]; 4] = [
        ["10001", "11011", "10101", "10001", "10001"], // M
        ["11110", "10001", "11110", "10001", "11110"], // B
        ["01111", "10000", "10000", "10000", "01111"], // C
        ["10010", "10010", "11111", "00010", "00010"], // 4
    ];
    let l = pen.lat(x, y, PITCH, CELL);
    for (n, g) in LETTERS.iter().enumerate() {
        let c = if n == 3 { hex(pal().acc) } else { hex(pal().ink) };
        for (r, row) in g.iter().enumerate() {
            for (cc, b) in row.chars().enumerate() {
                if b == '1' {
                    pen.cell(&l, n * 6 + cc, r, c);
                }
            }
        }
    }
}

// ───────────────────────────────── spectrum analyser

const FFT: usize = 2048;
/// analyser points (the drawn cell count follows the UI scale)
const NUM_COLS: usize = 225;
const TILT_DB_PER_OCT: f32 = 3.0;
const SILENT_DB: f32 = MIN_DB - 20.0;

struct Spectrum {
    last_head: usize,
    new_samples: usize,
    /// per analyser column, dB
    display: [f32; NUM_COLS],
}

impl Spectrum {
    fn new(head: usize) -> Self {
        Self { last_head: head, new_samples: 0, display: [SILENT_DB; NUM_COLS] }
    }
    /// at a fractional analyser column
    fn at(&self, idx: f32) -> f32 {
        let i = idx.clamp(0.0, (NUM_COLS - 1) as f32);
        let i0 = i as usize;
        let i1 = (i0 + 1).min(NUM_COLS - 1);
        self.display[i0] + (i - i0 as f32) * (self.display[i1] - self.display[i0])
    }
}

struct Analyzer {
    fft: Arc<dyn RealToComplex<f32>>,
    buf: Vec<f32>,
    raw: Vec<f32>,
    spec: Vec<realfft::num_complex::Complex<f32>>,
    scratch: Vec<realfft::num_complex::Complex<f32>>,
    win: Vec<f32>,
    col_hz: [f32; NUM_COLS],
    frame: [f32; NUM_COLS],
    pre: Spectrum,
    post: Spectrum,
}

impl Analyzer {
    fn new(sh: &Shared) -> Self {
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(FFT);
        let spec = fft.make_output_vec();
        let scratch = fft.make_scratch_vec();
        // Hann, not normalised (JUCE's windowing table)
        let win = (0..FFT).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (FFT - 1) as f32).cos()).collect();
        let col_hz = std::array::from_fn(|c| (MIN_HZ * (MAX_HZ / MIN_HZ).powf((c as f64 + 0.5) / NUM_COLS as f64)) as f32);
        Self {
            fft,
            buf: vec![0.0; FFT],
            raw: vec![0.0; FFT],
            spec,
            scratch,
            win,
            col_hz,
            frame: [0.0; NUM_COLS],
            pre: Spectrum::new(sh.pre.head()),
            post: Spectrum::new(sh.post.head()),
        }
    }

    /// The pre / post spectrum (always both): pull new samples, analyse when there are enough.
    fn update(&mut self, sh: &Shared) {
        let sr = sh.sample_rate.load(std::sync::atomic::Ordering::Relaxed) as f32;
        self.analyse(&sh.pre, true, true, sr);
        self.analyse(&sh.post, false, true, sr);
    }

    fn analyse(&mut self, ring: &Ring, pre: bool, on: bool, sr: f32) {
        let head = ring.head();
        let s = if pre { &mut self.pre } else { &mut self.post };
        let got = head.wrapping_sub(s.last_head).min(RING);
        s.last_head = head;
        if !on {
            return;
        }
        s.new_samples += got;
        if s.new_samples < FFT / 8 || sr <= 0.0 {
            // no fresh audio: let the spectrum fall
            for v in &mut s.display {
                *v = (*v - 0.75).max(SILENT_DB);
            }
            return;
        }
        s.new_samples = 0;

        ring.latest(&mut self.raw);
        for i in 0..FFT {
            self.buf[i] = self.raw[i] * self.win[i];
        }
        let _ = self.fft.process_with_scratch(&mut self.buf, &mut self.spec, &mut self.scratch);

        let bin_hz = sr / FFT as f32;
        // hann: sine amplitude → 1.0
        let norm = 4.0 / FFT as f32;
        let max_bin = FFT / 2 - 1;
        let mag = |b: usize| self.spec[b].norm();
        // 1/6-octave smoothing: average the power of all bins within ±1/12 octave
        let half_width = 2f32.powf(1.0 / 12.0);

        for i in 0..NUM_COLS {
            let f = self.col_hz[i];
            let (p_lo, p_hi) = (f / half_width / bin_hz, f * half_width / bin_hz);
            let power = if p_hi - p_lo < 2.0 {
                // narrower than a bin (low end): interpolate
                let pos = (f / bin_hz).clamp(0.0, max_bin as f32);
                let b0 = pos as usize;
                let b1 = (b0 + 1).min(max_bin);
                let t = pos - b0 as f32;
                let m = mag(b0) + t * (mag(b1) - mag(b0));
                m * m
            } else {
                let b0 = (p_lo.ceil() as usize).min(max_bin);
                let b1 = (p_hi as usize).clamp(b0, max_bin);
                (b0..=b1).map(|b| mag(b) * mag(b)).sum::<f32>() / (b1 - b0 + 1) as f32
            };
            let amp = power.sqrt() * norm;
            let db = if amp > 0.0 { (20.0 * amp.log10()).max(-140.0) } else { -140.0 };
            self.frame[i] = db + TILT_DB_PER_OCT * (f / 1000.0).log2();
        }

        // light smoothing across neighbouring columns, then ballistics
        let s = if pre { &mut self.pre } else { &mut self.post };
        for i in 0..NUM_COLS {
            let (mut acc, mut wsum) = (0.0, 0.0);
            for k in -2i32..=2 {
                let j = (i as i32 + k).clamp(0, NUM_COLS as i32 - 1) as usize;
                let wgt = (3 - k.abs()) as f32;
                acc += self.frame[j] * wgt;
                wsum += wgt;
            }
            let db = acc / wsum;
            let v = &mut s.display[i];
            // per 60 Hz tick: a soft rise and a slow fall, so the cells glide instead of jumping
            *v += (db - *v) * if db > *v { 0.25 } else { 0.06 };
        }
    }
}

// ───────────────────────────────── meters

#[derive(Clone, Copy)]
struct Meter {
    shown: f32,
    hold: f32,
    hold_time: i32,
}

impl Meter {
    const fn new() -> Self {
        Self { shown: -100.0, hold: -100.0, hold_time: 0 }
    }
    fn update(&mut self, v: f32) {
        self.shown = if v > self.shown { v } else { v.max(self.shown - 0.6) };
        // held value: stays 1.5 s, then falls slowly so it can be read
        if v >= self.hold {
            self.hold = v;
            self.hold_time = 90;
        } else {
            self.hold_time -= 1;
            if self.hold_time < 0 {
                self.hold = v.max(self.hold - 0.3);
            }
        }
    }
}

/// GR line: 0 .. 24 dB over GR_LEN
const GR_METER_RANGE: f32 = 24.0;
const LEARN_SECONDS: f64 = 3.0;
/// display ticks per second (the JUCE editor timer)
const TICK_HZ: f32 = 60.0;

#[derive(Clone, Copy, Default)]
struct GrMeter {
    /// positive dB, refreshed every display tick (60 Hz), the bar and its number alike
    shown: f32,
}

// ───────────────────────────────── editor state

struct Drag {
    hit: Hit,
    id: u32,
    anchor: Pos2,
    anchor_norm: f64,
    fine: bool,
    /// threshold drag: unquantised value and last y
    raw: f64,
    last_y: f32,
    /// resize: start pointer (points) and start width
    start: Pos2,
    w0: f32,
}

struct Edit {
    rect: [f32; 4],
    text: String,
    target: EditTarget,
    /// the band whose header text is being edited
    owner: Option<usize>,
    align: egui::Align,
    fresh: bool,
}

pub struct Mbc4Ui {
    params: Arc<Mbc4Params>,
    faders: Vec<Fader>,
    infos: HashMap<u32, (ParamRange, f64)>,
    fonts_ready: bool,
    light: bool,
    jp_font: bool,
    an: Analyzer,
    gr_disp: [f32; NUM_BANDS],
    gr_meters: [GrMeter; NUM_BANDS],
    in_meter: Meter,
    out_meter: Meter,
    tick_t: Option<Instant>,
    tick_acc: f32,
    drag: Option<Drag>,
    hot: Option<Hit>,
    mouse: Pos2,
    hover: Option<(Hit, Instant)>,
    edit: Option<Edit>,
    last_req: Option<(u32, u32)>,
    asked_size: bool,
    /// undo history: a settled change made here is one step
    ids: Vec<u32>,
    undo: Vec<Vec<f64>>,
    redo: Vec<Vec<f64>>,
    committed: Option<Vec<f64>>,
    ui_dirty: bool,
    last_wheel: Option<Instant>,
    /// the shared settings file, re-read when another plugin changes it
    theme_mtime: Option<SystemTime>,
    theme_check: Option<Instant>,
}

impl Mbc4Ui {
    fn new(params: Arc<Mbc4Params>) -> Self {
        let infos: HashMap<u32, (ParamRange, f64)> =
            params.param_infos().into_iter().map(|i| (i.id, (i.range.clone(), i.default_plain))).collect();
        let mut ids: Vec<u32> = infos.keys().copied().collect();
        ids.sort_unstable();
        let an = Analyzer::new(&params.shared);
        Self {
            params,
            faders: faders(),
            infos,
            fonts_ready: false,
            light: false,
            jp_font: false,
            an,
            gr_disp: [0.0; NUM_BANDS],
            gr_meters: [GrMeter::default(); NUM_BANDS],
            in_meter: Meter::new(),
            out_meter: Meter::new(),
            tick_t: None,
            tick_acc: 0.0,
            drag: None,
            hot: None,
            mouse: pos2(-100.0, -100.0),
            hover: None,
            edit: None,
            last_req: None,
            asked_size: false,
            ids,
            undo: Vec::new(),
            redo: Vec::new(),
            committed: None,
            ui_dirty: false,
            last_wheel: None,
            theme_mtime: None,
            theme_check: None,
        }
    }

    // ── params

    fn plain(&self, st: &Ctx, id: u32) -> f64 {
        f64::from(st.get_param_plain(id))
    }
    fn on(&self, st: &Ctx, id: impl Into<u32>) -> bool {
        self.plain(st, id.into()) > 0.5
    }
    fn norm(&self, id: u32, plain: f64) -> f64 {
        self.infos.get(&id).map_or(0.0, |(r, _)| r.normalize(plain))
    }
    fn denorm(&self, id: u32, n: f64) -> f64 {
        self.infos.get(&id).map_or(0.0, |(r, _)| r.denormalize(n.clamp(0.0, 1.0)))
    }
    fn bounds(&self, id: u32) -> (f64, f64) {
        self.infos.get(&id).map_or((0.0, 1.0), |(r, _)| (r.min(), r.max()))
    }
    fn set(&mut self, st: &Ctx, id: u32, plain: f64) {
        st.set_param(id, self.norm(id, plain));
        self.ui_dirty = true;
    }
    /// one complete host gesture
    fn set_once(&mut self, st: &Ctx, id: u32, plain: f64) {
        st.begin_edit(id);
        self.set(st, id, plain);
        st.end_edit(id);
    }
    fn default_of(&self, id: u32) -> f64 {
        self.infos.get(&id).map_or(0.0, |i| i.1)
    }

    // ── history: one drag, one wheel gesture or one click is one undo step

    fn snapshot(&self, st: &Ctx) -> Vec<f64> {
        self.ids.iter().map(|id| self.plain(st, *id)).collect()
    }
    fn commit(&mut self, st: &Ctx) {
        if self.drag.is_some() || self.edit.is_some() || self.last_wheel.is_some_and(|t| t.elapsed().as_millis() < 400) {
            return;
        }
        let now = self.snapshot(st);
        match &self.committed {
            None => self.committed = Some(now),
            Some(c) if *c == now => {}
            Some(c) => {
                // host-side changes (automation, presets) move the baseline but are not undo steps
                if self.ui_dirty {
                    self.undo.push(c.clone());
                    if self.undo.len() > 100 {
                        self.undo.remove(0);
                    }
                    self.redo.clear();
                }
                self.committed = Some(now);
            }
        }
        self.ui_dirty = false;
    }
    fn restore(&mut self, st: &Ctx, snap: Vec<f64>) {
        let cur = self.snapshot(st);
        for (i, id) in self.ids.clone().into_iter().enumerate() {
            if (cur[i] - snap[i]).abs() > 1e-9 {
                self.set_once(st, id, snap[i]);
            }
        }
        self.committed = Some(snap);
        self.ui_dirty = false;
    }
    fn do_undo(&mut self, st: &Ctx) {
        self.commit(st);
        if let Some(s) = self.undo.pop() {
            if let Some(c) = self.committed.take() {
                self.redo.push(c);
            }
            self.restore(st, s);
        }
    }
    fn do_redo(&mut self, st: &Ctx) {
        self.commit(st);
        if let Some(s) = self.redo.pop() {
            if let Some(c) = self.committed.take() {
                self.undo.push(c);
            }
            self.restore(st, s);
        }
    }
    /// the footer's reset: every parameter back to its default
    fn reset_all(&mut self, st: &Ctx) {
        for id in self.ids.clone() {
            let d = self.default_of(id);
            if (self.plain(st, id) - d).abs() > 1e-9 {
                self.set_once(st, id, d);
            }
        }
    }
    /// Alt + click / double-click: the control under the cursor back to its default
    fn reset_one(&mut self, st: &Ctx, h: Hit, p: Pos2) {
        let id = match h {
            Hit::Fader(i) => self.faders[i].id,
            Hit::Xo(i) => XO_IDS[i].into(),
            Hit::Thresh(b) => bid(b, THR),
            Hit::Plot => bid(self.band_at(st, p.x), THR),
            _ => return,
        };
        self.set_once(st, id, self.default_of(id));
    }
    /// a preset: ask the host for that size, aspect ratio kept
    fn request_scale(&mut self, st: &Ctx, z: f32) {
        let (rw, rh) = ((self.w() * z).round() as u32, (H * z).round() as u32);
        self.last_req = Some((rw, rh));
        let _ = st.request_resize(rw, rh);
    }
    fn io_open(&self) -> bool {
        self.params.io_open.load() != 0
    }
    /// design width of the current layout
    fn w(&self) -> f32 {
        width_of(self.io_open())
    }
    /// open / fold the INPUT / OUTPUT panel: the window grows or shrinks on the right, same scale
    fn toggle_io(&mut self, st: &Ctx, s: f32) {
        let open = !self.io_open();
        self.params.io_open.store(u32::from(open));
        let (rw, rh) = ((width_of(open) * s).round() as u32, (H * s).round() as u32);
        self.last_req = Some((rw, rh));
        let _ = st.request_resize(rw, rh);
    }
    fn bypassed(&self, st: &Ctx, b: usize) -> bool {
        self.on(st, bid(b, BYP))
    }
    fn xo(&self, st: &Ctx, i: usize) -> f64 {
        self.plain(st, XO_IDS[i].into())
    }
    /// band edge: below crossover 0 it is 20 Hz, above crossover 2 20 kHz
    fn edge_hz(&self, st: &Ctx, crossover: i32) -> f64 {
        match crossover {
            ..0 => MIN_HZ,
            3.. => MAX_HZ,
            c => self.xo(st, c as usize),
        }
    }
    fn band_at(&self, st: &Ctx, x: f32) -> usize {
        (0..3).find(|&b| x < freq_to_x(self.xo(st, b))).unwrap_or(3)
    }
    fn band_left_x(&self, st: &Ctx, b: usize) -> f32 {
        if b == 0 { PLOT_L } else { freq_to_x(self.xo(st, b - 1)) }
    }
    fn band_right_x(&self, st: &Ctx, b: usize) -> f32 {
        if b == 3 { PLOT_R } else { freq_to_x(self.xo(st, b)) }
    }
    fn xo_limits(&self, st: &Ctx, i: usize) -> (f64, f64) {
        let lo = if i > 0 { self.xo(st, i - 1) * 1.19 } else { MIN_HZ };
        let hi = if i < 2 { self.xo(st, i + 1) / 1.19 } else { MAX_HZ };
        (lo, hi.max(lo))
    }

    /// plain drags and the wheel: steps or the list; fine: the fine step
    fn snap(&self, f: &Fader, v: f64, fine: bool) -> f64 {
        let (lo, hi) = self.bounds(f.id);
        let v = v.clamp(lo, hi);
        if fine {
            return ((v / f.steps.fine).round() * f.steps.fine).clamp(lo, hi);
        }
        if !f.steps.list.is_empty() {
            let n = self.norm(f.id, v);
            return f.steps.list.iter().copied().fold(f.steps.list[0], |best, s| {
                if (self.norm(f.id, s) - n).abs() < (self.norm(f.id, best) - n).abs() { s } else { best }
            });
        }
        ((v / f.steps.step).round() * f.steps.step).clamp(lo, hi)
    }

    // ── band header geometry

    fn range_lo(&self, st: &Ctx, b: usize) -> String {
        hz_short(self.edge_hz(st, b as i32 - 1))
    }
    fn range_hi(&self, st: &Ctx, b: usize) -> String {
        hz_short(self.edge_hz(st, b as i32))
    }
    fn lo_text_rect(&self, pen: &Pen<'_>, st: &Ctx, b: usize) -> [f32; 4] {
        [strip_x(b) + RANGE_X - 2.0, HEADER_Y - 7.0, pen.text_w(&self.range_lo(st, b), VAL) + 4.0, 14.0]
    }
    fn hi_text_rect(&self, pen: &Pen<'_>, st: &Ctx, b: usize) -> [f32; 4] {
        let x = strip_x(b) + RANGE_X + pen.text_w(&format!("{}{DASH}", self.range_lo(st, b)), VAL);
        [x - 2.0, HEADER_Y - 7.0, pen.text_w(&self.range_hi(st, b), VAL) + 4.0, 14.0]
    }

    fn hit(&self, pen: &Pen<'_>, st: &Ctx, p: Pos2) -> Option<Hit> {
        let open = self.io_open();
        if in_r(IO_RECT, p) {
            return Some(Hit::Io);
        }
        for (i, f) in self.faders.iter().enumerate() {
            // the panel's controls only while it is open
            if f.band.is_none() && !open {
                continue;
            }
            if in_r(f.rect(), p) || (f.on_meter && in_r(f.value_rect(), p)) {
                return Some(Hit::Fader(i));
            }
        }
        for b in 0..NUM_BANDS {
            for j in 0..3 {
                if in_r(toggle_rect(b, j), p) {
                    return Some(Hit::Toggle(b, j));
                }
            }
            if in_r(match_rect(b), p) {
                return Some(Hit::Match(b));
            }
            if b > 0 && in_r(self.lo_text_rect(pen, st, b), p) {
                return Some(Hit::BandLo(b));
            }
            if b < 3 && in_r(self.hi_text_rect(pen, st, b), p) {
                return Some(Hit::BandHi(b));
            }
        }
        let fr = frame_r(open);
        for (i, r) in zoom_rects(pen, fr).into_iter().enumerate() {
            if in_r(r, p) {
                return Some(Hit::Zoom(i));
            }
        }
        for i in 0..2 {
            if in_r(icon_rect(pen, fr, i), p) {
                return Some(Hit::Icon(i));
            }
        }
        let w = width_of(open);
        if in_r([w - 16.0, H - 16.0, 16.0, 16.0], p) {
            return Some(Hit::Resize);
        }
        // the graph: crossover lines, then the band's threshold, then the plot
        if p.y >= PLOT_T && p.y <= PLOT_B {
            let mut best = None;
            let mut best_d = 4.5;
            for i in 0..3 {
                let d = (freq_to_x(self.xo(st, i)) - p.x).abs();
                if d < best_d {
                    best_d = d;
                    best = Some(Hit::Xo(i));
                }
            }
            if best.is_some() {
                return best;
            }
        }
        if p.x >= PLOT_L && p.x < PLOT_R && p.y >= PLOT_T && p.y < PLOT_B {
            let b = self.band_at(st, p.x);
            let thr = self.plain(st, bid(b, THR)) as f32;
            return Some(if (p.y - db_to_y(thr)).abs() <= 4.0 { Hit::Thresh(b) } else { Hit::Plot });
        }
        None
    }

    fn tip_of(&self, h: Hit) -> Option<Tip> {
        Some(match h {
            Hit::Fader(i) => self.faders[i].tip,
            Hit::Toggle(_, 0) => Tip::Solo,
            Hit::Toggle(_, 1) => Tip::Mute,
            Hit::Toggle(..) => Tip::Bypass,
            Hit::Match(_) => Tip::Match,
            Hit::Icon(0) => Tip::Reset,
            Hit::Icon(_) => Tip::Theme,
            Hit::Io => Tip::Io,
            Hit::Zoom(_) => Tip::Zoom,
            // the graph shows bubbles instead
            Hit::Xo(_) | Hit::Thresh(_) | Hit::Plot | Hit::BandLo(_) | Hit::BandHi(_) | Hit::Resize => return None,
        })
    }

    // ── input

    #[allow(clippy::too_many_lines)]
    fn input(&mut self, ctx: &egui::Context, pen: &Pen<'_>, st: &Ctx) {
        let (pos, pressed, down, released, dbl, fine, alt, wheel, keys) = ctx.input(|i| {
            let m = i.modifiers;
            let wheel: Vec<f32> = i
                .raw
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::MouseWheel { delta, .. } => {
                        let d = if delta.y.abs() > delta.x.abs() { delta.y } else { -delta.x };
                        (d != 0.0).then_some(d)
                    }
                    _ => None,
                })
                .collect();
            (
                i.pointer.latest_pos(),
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.primary_released(),
                i.pointer.button_double_clicked(egui::PointerButton::Primary),
                m.shift || m.ctrl,
                m.alt,
                wheel,
                (m.command, m.shift, i.key_pressed(egui::Key::Z), i.key_pressed(egui::Key::Y)),
            )
        });
        // undo / redo live on the keyboard only
        let (cmd, shift, z, y) = keys;
        if cmd && z && !shift {
            self.do_undo(st);
        } else if cmd && ((z && shift) || y) {
            self.do_redo(st);
        }
        let Some(pos) = pos else {
            if self.drag.is_none() {
                self.hot = None;
            }
            return;
        };
        let (o, s) = (pen.o, pen.s);
        let p = pos2((pos.x - o.x) / s, (pos.y - o.y) / s);
        self.mouse = p;

        if let Some(d) = &self.drag {
            if released || !down {
                if d.hit != Hit::Resize {
                    st.end_edit(d.id);
                }
                self.drag = None;
                // a double click still lands below (it arrives with the release)
            } else {
                if d.hit == Hit::Resize {
                    // the corner: ask the host for a new size, aspect ratio kept
                    let ww = self.w();
                    let w = (d.w0 + (pos.x - d.start.x).max((pos.y - d.start.y) * ww / H)).clamp(ww * SCALE_RANGE.0, ww * SCALE_RANGE.1);
                    let (rw, rh) = (w.round() as u32, (w * H / ww).round() as u32);
                    if self.last_req != Some((rw, rh)) {
                        self.last_req = Some((rw, rh));
                        let _ = st.request_resize(rw, rh);
                    }
                } else {
                    self.apply_drag(st, p, fine);
                }
                return;
            }
        }

        let h = self.hit(pen, st, p);
        self.hot = h;
        match self.hover {
            Some((hh, _)) if Some(hh) == h => {}
            _ => self.hover = h.map(|hh| (hh, Instant::now())),
        }
        ctx.set_cursor_icon(match h {
            Some(Hit::Fader(i)) if !self.faders[i].vertical => egui::CursorIcon::ResizeHorizontal,
            Some(Hit::Xo(_)) => egui::CursorIcon::ResizeHorizontal,
            Some(Hit::Fader(_) | Hit::Thresh(_) | Hit::Plot) => egui::CursorIcon::ResizeVertical,
            Some(Hit::Resize) => egui::CursorIcon::ResizeNwSe,
            Some(Hit::Toggle(..) | Hit::Match(_) | Hit::Icon(_) | Hit::Zoom(_) | Hit::Io) => egui::CursorIcon::PointingHand,
            Some(Hit::BandLo(_) | Hit::BandHi(_)) => egui::CursorIcon::Text,
            None => egui::CursorIcon::Default,
        });

        if dbl && let Some(h) = h {
            self.hover = None;
            match h {
                Hit::Fader(i) => {
                    let f = self.faders[i];
                    if in_r(f.value_rect(), p) {
                        let v = self.plain(st, f.id);
                        let align = if f.vertical { egui::Align::Center } else { egui::Align::Max };
                        self.edit = Some(Edit { rect: f.value_rect(), text: number_text(v), target: EditTarget::Fader(i), owner: None, align, fresh: true });
                    } else {
                        self.reset_one(st, h, p);
                    }
                }
                Hit::Xo(_) | Hit::Thresh(_) | Hit::Plot => self.reset_one(st, h, p),
                Hit::BandLo(b) => self.open_xo_edit(st, b, b - 1, self.lo_text_rect(pen, st, b)),
                Hit::BandHi(b) => self.open_xo_edit(st, b, b, self.hi_text_rect(pen, st, b)),
                _ => {}
            }
            return;
        }

        if pressed && let Some(h) = h {
            self.hover = None;
            if alt {
                if matches!(h, Hit::Zoom(_)) {
                    self.request_scale(st, DEFAULT_SCALE);
                } else {
                    self.reset_one(st, h, p);
                }
                return;
            }
            match h {
                Hit::Fader(i) => {
                    let f = self.faders[i];
                    st.begin_edit(f.id);
                    let anchor_norm = self.norm(f.id, self.plain(st, f.id));
                    self.drag = Some(Drag { hit: h, id: f.id, anchor: p, anchor_norm, fine, raw: 0.0, last_y: p.y, start: pos, w0: 0.0 });
                }
                Hit::Xo(i) => {
                    let id: u32 = XO_IDS[i].into();
                    st.begin_edit(id);
                    self.drag = Some(Drag { hit: h, id, anchor: p, anchor_norm: 0.0, fine, raw: 0.0, last_y: p.y, start: pos, w0: 0.0 });
                }
                // anywhere inside a band moves its threshold
                Hit::Thresh(_) | Hit::Plot => {
                    let b = self.band_at(st, p.x);
                    let id = bid(b, THR);
                    st.begin_edit(id);
                    let raw = self.plain(st, id);
                    self.drag = Some(Drag { hit: Hit::Thresh(b), id, anchor: p, anchor_norm: 0.0, fine, raw, last_y: p.y, start: pos, w0: 0.0 });
                }
                Hit::Toggle(b, j) => {
                    let id = bid(b, SOLO + j);
                    let v = if self.on(st, id) { 0.0 } else { 1.0 };
                    self.set_once(st, id, v);
                }
                Hit::Match(b) => self.toggle_learn(st, b),
                Hit::Icon(0) => self.reset_all(st),
                Hit::Icon(_) => {
                    let dark = !DARK.load(Relaxed);
                    DARK.store(dark, Relaxed);
                    save_theme(dark);
                    self.theme_mtime = settings_mtime();
                }
                Hit::Zoom(i) => self.request_scale(st, ZOOMS[i].0),
                Hit::Io => self.toggle_io(st, s),
                Hit::Resize => {
                    self.last_req = None;
                    self.drag = Some(Drag { hit: h, id: u32::MAX, anchor: p, anchor_norm: 0.0, fine: false, raw: 0.0, last_y: p.y, start: pos, w0: self.w() * s });
                }
                Hit::BandLo(_) | Hit::BandHi(_) => {}
            }
            return;
        }

        if let Some(Hit::Fader(i)) = h {
            for d in wheel {
                self.last_wheel = Some(Instant::now());
                self.wheel(st, i, if d > 0.0 { 1 } else { -1 }, fine);
            }
        }
    }

    fn wheel(&mut self, st: &Ctx, i: usize, dir: i32, fine: bool) {
        let f = self.faders[i];
        let value = self.plain(st, f.id);
        let v = if !f.steps.list.is_empty() && !fine {
            let cur = self.snap(&f, value, false);
            let idx = f.steps.list.iter().position(|s| *s == cur).unwrap_or(0) as i32;
            f.steps.list[(idx + dir).clamp(0, f.steps.list.len() as i32 - 1) as usize]
        } else {
            self.snap(&f, value + f64::from(dir) * if fine { f.steps.fine } else { f.steps.step }, fine)
        };
        if v != value {
            self.set_once(st, f.id, v);
        }
    }

    fn apply_drag(&mut self, st: &Ctx, p: Pos2, fine: bool) {
        let Some(hit) = self.drag.as_ref().map(|d| d.hit) else { return };
        match hit {
            Hit::Fader(i) => {
                let f = self.faders[i];
                // the anchor moves whenever the fine modifier changes
                if self.drag.as_ref().is_some_and(|d| d.fine != fine) {
                    let n = self.norm(f.id, self.plain(st, f.id));
                    if let Some(d) = &mut self.drag {
                        d.anchor = p;
                        d.anchor_norm = n;
                        d.fine = fine;
                    }
                }
                let Some(d) = &self.drag else { return };
                let delta = if f.vertical { (d.anchor.y - p.y) / 160.0 } else { (p.x - d.anchor.x) / 128.0 };
                let n = (d.anchor_norm + f64::from(delta) * if fine { 0.2 } else { 1.0 }).clamp(0.0, 1.0);
                let v = self.snap(&f, self.denorm(f.id, n), fine);
                if v != self.plain(st, f.id) {
                    self.set(st, f.id, v);
                }
            }
            Hit::Xo(i) => {
                let (lo, hi) = self.xo_limits(st, i);
                let f = round_significant(x_to_freq(p.x).clamp(MIN_HZ, MAX_HZ), if fine { 3 } else { 2 });
                self.set(st, XO_IDS[i].into(), f.clamp(lo, hi));
            }
            Hit::Thresh(b) => {
                // incremental, so the fine modifier can be pressed mid-drag
                let px_per_db = (PLOT_B - PLOT_T) / (MAX_DB - MIN_DB);
                let Some(d) = &mut self.drag else { return };
                d.raw = (d.raw - f64::from((p.y - d.last_y) / px_per_db) * if fine { 0.2 } else { 1.0 }).clamp(-60.0, 0.0);
                d.last_y = p.y;
                let step = if fine { 0.1 } else { 1.0 };
                let v = (d.raw / step).round() * step;
                self.set(st, bid(b, THR), v);
            }
            _ => {}
        }
    }

    fn open_xo_edit(&mut self, st: &Ctx, band: usize, index: usize, r: [f32; 4]) {
        let rect = [r[0], r[1], r[2].max(44.0), r[3]];
        self.edit = Some(Edit { rect, text: hz_short(self.xo(st, index)), target: EditTarget::Xo(index), owner: Some(band), align: egui::Align::Min, fresh: true });
    }

    fn apply_edit(&mut self, st: &Ctx, e: &Edit) {
        match e.target {
            EditTarget::Fader(i) => {
                let f = self.faders[i];
                let t: String = e.text.chars().filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-').collect();
                if let Ok(v) = t.parse::<f64>() {
                    let (lo, hi) = self.bounds(f.id);
                    self.set_once(st, f.id, v.clamp(lo, hi));
                }
            }
            EditTarget::Xo(i) => {
                if let Some(f) = hz_from_text(&e.text).filter(|f| *f > 0.0) {
                    let (lo, hi) = self.xo_limits(st, i);
                    self.set_once(st, XO_IDS[i].into(), f.clamp(lo, hi));
                }
            }
        }
    }

    // ── gain match

    fn toggle_learn(&mut self, st: &Ctx, b: usize) {
        if self.bypassed(st, b) {
            return;
        }
        let sh = &self.params.shared;
        if sh.is_learning(b) { sh.stop_learning(b) } else { sh.start_learning(b) }
    }

    fn update_learn(&mut self, st: &Ctx, b: usize) {
        let sh = &self.params.shared;
        if sh.is_learning(b) && sh.learned_seconds(b) >= LEARN_SECONDS {
            // done: store the compensation once; it does not move afterwards
            sh.stop_learning(b);
            let avg = f64::from(sh.learned_average_db(b));
            self.set_once(st, bid(b, GAIN), (-avg).clamp(0.0, 24.0));
        }
    }

    // ── display ticks (30 Hz, like the JUCE editor's timer)

    fn tick(&mut self, st: &Ctx) {
        let sh = self.params.shared.clone();
        self.an.update(&sh);
        for b in 0..NUM_BANDS {
            let gr = sh.gr_db(b);
            let d = &mut self.gr_disp[b];
            *d = if gr < *d { *d + (gr - *d) * 0.5 } else { *d + (gr - *d) * 0.12 };

            let byp = self.bypassed(st, b);
            let m = &mut self.gr_meters[b];
            let g = if byp { 0.0 } else { (-gr).min(GR_METER_RANGE) }; // positive dB
            // the compressor's own attack / release already smooth it: show every tick as it is
            m.shown = g;
            self.update_learn(st, b);
        }
        self.in_meter.update(sh.in_peak_db());
        self.out_meter.update(sh.out_peak_db());
    }

    // ── drawing

    fn draw(&mut self, pen: &Pen<'_>, st: &Ctx) {
        // no dividers: the blocks are set apart by the grid alone
        let open = self.io_open();

        // the logo takes the main block's heading place
        logo(pen, COL_L + 10.0, 25.5);
        self.draw_io_button(pen);
        if open {
            // INPUT | OUTPUT above, LOOK and MIX below (level with the band strips)
            pen.section_header("INPUT", COL_R + 10.0, 65.0, hex(pal().ink));
            pen.section_header("OUTPUT", OUT_COL + 10.0, 65.0, hex(pal().ink));
            self.draw_meter(pen, IN_METER_X, self.in_meter);
            self.draw_meter(pen, OUT_METER_X, self.out_meter);
        }
        self.draw_footer(pen);

        self.draw_graph(pen, st);
        for b in 0..NUM_BANDS {
            self.draw_strip(pen, st, b);
        }
        for i in 0..self.faders.len() {
            if self.faders[i].band.is_none() && !open {
                continue;
            }
            self.draw_fader(pen, st, i);
        }
        self.draw_bubbles(pen, st);
    }

    /// I/O: a letter button at the right end of the top row; orange while the panel is open
    fn draw_io_button(&self, pen: &Pen<'_>) {
        let r = IO_RECT;
        let c = if self.io_open() {
            pal().acc
        } else if self.hot == Some(Hit::Io) {
            pal().ink
        } else {
            pal().ink3
        };
        pen.text("I/O", r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, LABEL, hex(c), Align2::CENTER_CENTER);
    }

    /// system bar: version on the left; reset, theme and the UI sizes on the right
    fn draw_footer(&self, pen: &Pen<'_>) {
        let fr = frame_r(self.io_open());
        pen.text(concat!("V", env!("CARGO_PKG_VERSION")), FRAME_L, FOOT_Y, NOTE, hex(pal().ink3), Align2::LEFT_CENTER);
        for i in 0..2 {
            let r = icon_rect(pen, fr, i);
            let c = if self.hot == Some(Hit::Icon(i)) { pal().ink } else { pal().ink3 };
            pen.icon(i, r[0] + r[2] / 2.0, FOOT_Y, hex(c));
        }
        // ink only when the window is exactly that preset; after a corner drag none is
        let uw = self.params.ui_width.load() as f32;
        for (i, r) in zoom_rects(pen, fr).into_iter().enumerate() {
            let on = (uw - W * ZOOMS[i].0).abs() <= 1.0 || self.hot == Some(Hit::Zoom(i));
            pen.text(ZOOMS[i].1, r[0] + r[2] / 2.0, FOOT_Y, NOTE, hex(if on { pal().ink } else { pal().ink3 }), Align2::CENTER_CENTER);
        }
    }

    /// two columns of dots on the graph's rows (same lattice); above 0 dB in orange
    fn draw_meter(&self, pen: &Pen<'_>, cx: f32, m: Meter) {
        let l = pen.lat(cx - 3.5, PLOT_T, PITCH, CELL);
        let rows = l.fit(PLOT_B - PLOT_T);
        let (n, r0) = (rows_for(m.shown, rows), rows_for(0.0, rows));
        for j in 0..rows {
            let c = if j >= n {
                hex(pal().off)
            } else if j >= r0 {
                hex(pal().acc)
            } else {
                hex(pal().kept)
            };
            pen.cell(&l, 0, rows - 1 - j, c);
            pen.cell(&l, 1, rows - 1 - j, c);
        }
        // 0 dB reference: exactly as wide as the dots
        pen.line_h(l.x(0), l.x(0) + l.ext(2), db_to_y(0.0), hex(pal().ink), 0.5);
        let readout = if m.hold <= MIN_DB {
            "--.-".to_owned()
        } else {
            format!("{}{:.1}", if m.hold > 0.0 { "+" } else { "" }, m.hold)
        };
        pen.text(&readout, cx, READOUT_TOP, VAL, if m.hold > 0.0 { hex(pal().acc) } else { hex(pal().ink) }, Align2::CENTER_CENTER);
    }

    /// "02 SPLIT": the spectrum drawn with cells, crossovers and per-band thresholds.
    /// Output spectrum in `kept` grey, the part each band's compressor removes in orange,
    /// the input behind it in a pale grey.
    fn draw_graph(&mut self, pen: &Pen<'_>, st: &Ctx) {
        self.draw_cells(pen, st);

        // scale: numbers only
        let mut db = 0.0;
        while db >= MIN_DB {
            pen.text(&format!("{db:.0}"), PLOT_R + 7.0, db_to_y(db), TICK, hex(pal().ink2), Align2::LEFT_CENTER);
            db -= 12.0;
        }
        for (f, t) in [(50.0, "50"), (100.0, "100"), (200.0, "200"), (500.0, "500"), (1000.0, "1k"), (2000.0, "2k"), (5000.0, "5k"), (10000.0, "10k")] {
            pen.text(t, freq_to_x(f), READOUT_TOP, TICK, hex(pal().ink2), Align2::CENTER_CENTER);
        }

        let hot = self.drag.as_ref().map(|d| d.hit).or(self.hot);
        for i in 0..3 {
            let t = if hot == Some(Hit::Xo(i)) { 1.2 } else { 0.5 };
            pen.line_v(freq_to_x(self.xo(st, i)), PLOT_T, PLOT_B, hex(pal().ink), t);
        }
        for b in 0..NUM_BANDS {
            let (x0, x1) = (self.band_left_x(st, b), self.band_right_x(st, b));
            let byp = self.bypassed(st, b);
            let thr = self.plain(st, bid(b, THR)) as f32;
            let t = if hot == Some(Hit::Thresh(b)) { 1.2 } else { 0.5 };
            pen.line_h(x0, x1, db_to_y(thr), if byp { hex(pal().d_knob) } else { hex(pal().ink) }, t);
            pen.text(LETTERS[b], x0 + 3.0, PLOT_T - 10.0, LABEL, if byp { hex(pal().d_text) } else { hex(pal().ink2) }, Align2::LEFT_CENTER);
            self.draw_gr(pen, b, x0 + 3.0, db_to_y(thr) + GR_BELOW, byp);
        }
    }

    fn draw_cells(&self, pen: &Pen<'_>, st: &Ctx) {
        // whole-pixel lattice: how many cells fit depends a little on the UI scale
        let l = pen.lat(PLOT_L, PLOT_T, PITCH, CELL);
        let (cols, rows) = (l.fit(PLOT_R - PLOT_L), l.fit(PLOT_B - PLOT_T));
        let (kept_i, acc_i, ghost_i, off_i) = (0, 1, 2, 3);
        let colour = |i: usize| match i {
            0 => hex(pal().kept),
            1 => hex(pal().acc),
            2 => hex(pal().ghost),
            _ => hex(pal().off),
        };
        let byp: [bool; NUM_BANDS] = std::array::from_fn(|b| self.bypassed(st, b));
        // wet share (band mix × global mix): below 100 % the removed part fades from its tip to its root
        let global = self.plain(st, P::Mix.into()) * 0.01;
        let wet: [f32; NUM_BANDS] = std::array::from_fn(|b| (self.plain(st, bid(b, MIX)) * 0.01 * global).clamp(0.0, 1.0) as f32);
        let (acc, bg) = (hex(pal().acc), hex(pal().cell));

        // runs of one colour are filled as single rectangles, the gaps drawn over them afterwards
        for c in 0..cols {
            let cx = l.x(c) + l.cell() * 0.5;
            let f = x_to_freq(cx);
            let idx = ((f / MIN_HZ).ln() / (MAX_HZ / MIN_HZ).ln()) as f32 * NUM_COLS as f32 - 0.5;
            let b = self.band_at(st, cx);
            let in_db = self.an.pre.at(idx);
            let gr = if byp[b] { 0.0 } else { self.gr_disp[b] };
            let n_in = rows_for(in_db, rows);
            let n_out = rows_for(self.an.post.at(idx), rows);
            let n_cut = rows_for(in_db + gr, rows);
            let colour_of = |j: usize| {
                if j < n_out {
                    kept_i
                } else if j < n_in && j >= n_cut {
                    acc_i // removed by the compressor
                } else if j < n_in {
                    ghost_i
                } else {
                    off_i
                }
            };
            let (mut run_start, mut run_c) = (0, colour_of(0));
            for j in 1..=rows {
                let cj = if j < rows { colour_of(j) } else { usize::MAX };
                if cj == run_c {
                    continue;
                }
                // rows [run_start, j) counted from the bottom
                if run_c == acc_i && wet[b] < 0.999 {
                    // opaque at the tip (the top of the cut), the wet share at its root
                    let (r0, r1) = (n_cut.max(n_out), n_in);
                    for jj in run_start..j {
                        let t = if r1 > r0 + 1 { (jj - r0) as f32 / (r1 - r0 - 1) as f32 } else { 1.0 };
                        let a = wet[b] + (1.0 - wet[b]) * t.clamp(0.0, 1.0);
                        pen.cell(&l, c, rows - 1 - jj, lerp_col(bg, acc, a));
                    }
                } else {
                    pen.span(&l, c, rows - j, 1, j - run_start, colour(run_c));
                }
                run_start = j;
                run_c = cj;
            }
        }
        // gaps between the cells of a run, in the background colour
        let bg = hex(pal().cell);
        let (top, height, left, width) = (l.y(0), l.ext(rows), l.x(0), l.ext(cols));
        for r in 0..rows.saturating_sub(1) {
            pen.fill(left, l.y(r) + l.cell(), width, l.gap(), bg);
        }
        for c in 0..cols.saturating_sub(1) {
            pen.fill(l.x(c) + l.cell(), top, l.gap(), height, bg);
        }
    }

    /// One band's header: letter, range, the GR bar, MATCH and S / M / B
    /// (the sliders are drawn with the other faders).
    fn draw_strip(&self, pen: &Pen<'_>, st: &Ctx, b: usize) {
        let x0 = strip_x(b);
        let dim = self.bypassed(st, b);
        let ink = if dim { hex(pal().d_text) } else { hex(pal().ink) };
        pen.section_header(LETTERS[b], x0 + 10.0, HEADER_Y, ink);
        if self.edit.as_ref().is_none_or(|e| e.owner != Some(b)) {
            let s = format!("{}{DASH}{} Hz", self.range_lo(st, b), self.range_hi(st, b));
            pen.text(&s, x0 + RANGE_X, HEADER_Y, VAL, if dim { hex(pal().d_text) } else { hex(pal().ink2) }, Align2::LEFT_CENTER);
        }

        // S / M / B: the letter is the button: orange when on, ink on hover, ink3 when off
        for (j, name) in ["S", "M", "B"].iter().enumerate() {
            let r = toggle_rect(b, j);
            let c = if self.on(st, bid(b, SOLO + j)) {
                pal().acc
            } else if self.hot == Some(Hit::Toggle(b, j)) {
                pal().ink
            } else {
                pal().ink3
            };
            pen.text(name, r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, LABEL, hex(c), Align2::CENTER_CENTER);
        }

        // gain match: a small push button in a rounded groove, orange text while learning
        let r = match_rect(b);
        pen.pill(r[0], r[1], r[2], r[3], hex(pal().off), None);
        let c = if dim {
            hex(pal().d_text)
        } else if self.params.shared.is_learning(b) {
            hex(pal().acc)
        } else {
            hex(pal().ink)
        };
        pen.text("MATCH", r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, LABEL, c, Align2::CENTER_CENTER);

    }

    /// A band's gain reduction under the left end of its threshold line: an orange line growing
    /// to the right, the number riding on its tip, both on a ground of the number's height.
    fn draw_gr(&self, pen: &Pen<'_>, b: usize, x: f32, cy: f32, dim: bool) {
        let m = self.gr_meters[b];
        let len = (m.shown / GR_METER_RANGE).clamp(0.0, 1.0) * GR_LEN;
        let tip = if len > 0.0 { x + len + 2.5 } else { x };
        let reducing = !dim && m.shown >= 0.05;
        let (t, c) = if reducing {
            (format!("-{:.1}", m.shown), hex(pal().acc))
        } else {
            ("0.0".to_owned(), if dim { hex(pal().d_text) } else { hex(pal().ink3) })
        };
        // a ground exactly the number's font height, from the line's left end to the number's right edge
        let end = tip + pen.text_w(&t, TINY);
        pen.rect(x, cy - TINY.size() / 2.0, end - x, TINY.size(), hex(pal().cell));
        if len > 0.0 {
            pen.line_h(x, x + len, cy, if dim { hex(pal().d_knob) } else { hex(pal().acc) }, GR_LINE);
        }
        pen.text(&t, tip, cy, TINY, c, Align2::LEFT_CENTER);
    }

    fn draw_fader(&self, pen: &Pen<'_>, st: &Ctx, i: usize) {
        let f = self.faders[i];
        let v = self.plain(st, f.id);
        let dim = f.band.is_some_and(|b| self.bypassed(st, b));
        let (lo_v, hi_v) = self.bounds(f.id);

        let n = f.cells();
        let r = (self.norm(f.id, v) * (n - 1) as f64).round() as usize;
        let bipolar = lo_v < 0.0 && hi_v > 0.0;
        let b0 = if bipolar { (self.norm(f.id, 0.0) * (n - 1) as f64).round() as usize } else { 0 };
        let (lo, hi) = (b0.min(r), b0.max(r));
        let cap = if dim { hex(pal().d_knob) } else { hex(pal().ink) };
        let lit = if dim { hex(pal().d_lit) } else { hex(pal().lit) };
        let text_c = if dim { hex(pal().d_text) } else { hex(pal().ink) };
        let editing = self.edit.as_ref().is_some_and(|e| e.target == EditTarget::Fader(i));

        if f.on_meter {
            // the knob is the value's row of the meter's own cells; the level stays visible around it
            let l = pen.lat(f.x - 3.5, PLOT_T, PITCH, CELL);
            let rows = l.fit(PLOT_B - PLOT_T);
            let r = (self.norm(f.id, v) * (rows - 1) as f64).round() as usize;
            pen.span(&l, 0, rows - 1 - r, 2, 1, cap);
            if !editing {
                pen.text(&format_value(v, f.format), f.x, METER_VALUE_Y, VAL, text_c, Align2::CENTER_CENTER);
            }
        } else if f.vertical {
            let l = pen.lat(f.x - 3.5, FADER_CELLS_Y, PITCH, CELL);
            for j in 0..n {
                let c = if j == r {
                    cap
                } else if j >= lo && j <= hi {
                    lit
                } else {
                    hex(pal().off)
                };
                pen.cell(&l, 0, n - 1 - j, c);
                pen.cell(&l, 1, n - 1 - j, c);
            }
            pen.text(f.label, f.x, FADER_LABEL_Y, LABEL, text_c, Align2::CENTER_CENTER);
            if !editing {
                pen.text(&format_value(v, f.format), f.x, FADER_VALUE_Y, VAL, text_c, Align2::CENTER_CENTER);
            }
        } else {
            let cy = f.y;
            let l = pen.lat(f.x + 40.0, cy - 1.5, PITCH, CELL);
            for j in 0..n {
                let c = if j == r {
                    cap
                } else if j < r {
                    lit
                } else {
                    hex(pal().off)
                };
                pen.cell(&l, j, 0, c);
            }
            pen.text(f.label, f.x + 4.0, cy, LABEL, text_c, Align2::LEFT_CENTER);
            if !editing {
                pen.text(&format_value(v, f.format), f.x + SLIDER_W - 4.0, cy, VAL, text_c, Align2::RIGHT_CENTER);
            }
        }
    }

    /// value bubble for whatever is under the mouse / being moved
    fn draw_bubbles(&self, pen: &Pen<'_>, st: &Ctx) {
        let h = self.drag.as_ref().map(|d| d.hit).or(self.hot);
        match h {
            Some(Hit::Thresh(b)) => {
                let thr = self.plain(st, bid(b, THR));
                pen.bubble(&format!("{}  {}", LETTERS[b], format_value(thr, Format::Db)), self.mouse.x, db_to_y(thr as f32) - 12.0, PLOT_L, PLOT_R);
            }
            Some(Hit::Xo(i)) => {
                let x = self.xo(st, i);
                pen.bubble(&hz_text(x), freq_to_x(x), PLOT_T + 10.0, PLOT_L, PLOT_R);
            }
            Some(Hit::Plot) => {
                let s = format!("{}  {} dB", hz_text(x_to_freq(self.mouse.x)), number_text(f64::from(y_to_db(self.mouse.y))));
                pen.bubble(&s, self.mouse.x, self.mouse.y - 14.0, PLOT_L, PLOT_R);
            }
            _ => {}
        }
    }

    /// tooltip text: heading (upper case, accent) and the Japanese body
    fn draw_tooltip(&self, ctx: &egui::Context, origin: Pos2, s: f32, screen: Rect) {
        let Some((h, since)) = self.hover else { return };
        if since.elapsed().as_millis() < 500 || self.drag.is_some() || self.edit.is_some() {
            return;
        }
        let Some(tip) = self.tip_of(h) else { return };
        let (head, body) = tip.text();
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("mbc4-tip")));
        let pad = vec2(9.0, 7.0) * s;
        let head_fmt = egui::TextFormat {
            font_id: FontId::new(8.0 * s, FontFamily::Monospace),
            color: hex(pal().acc),
            extra_letter_spacing: 0.8 * s,
            ..Default::default()
        };
        let g_head = painter.layout_job(egui::text::LayoutJob::single_section(head.to_uppercase(), head_fmt));
        let mut body_job = egui::text::LayoutJob::single_section(
            body.to_owned(),
            egui::TextFormat {
                font_id: FontId::new(10.5 * s, FontFamily::Proportional),
                color: hex(pal().tip_text),
                line_height: Some(10.5 * s * 1.55),
                ..Default::default()
            },
        );
        body_job.wrap.max_width = 220.0 * s;
        let g_body = painter.layout_job(body_job);
        let size = vec2(g_head.size().x.max(g_body.size().x), g_head.size().y + 4.0 * s + g_body.size().y + 2.0) + pad * 2.0;
        let m = pos2(origin.x + self.mouse.x * s, origin.y + self.mouse.y * s);
        let gap = 14.0 * s;
        let mut p = m + vec2(gap, gap);
        if p.x + size.x > screen.right() {
            p.x = m.x - size.x - gap / 2.0;
        }
        if p.y + size.y > screen.bottom() {
            p.y = m.y - size.y - gap / 2.0;
        }
        p = p.max(screen.min);
        let r = Rect::from_min_size(p, size);
        let (bg, a) = pal().tip_bg;
        painter.rect_filled(r, (3.0 * s) as u8, Color32::from_rgba_unmultiplied((bg >> 16) as u8, (bg >> 8) as u8, bg as u8, a));
        let y = p.y + pad.y;
        painter.galley(pos2(p.x + pad.x, y), g_head.clone(), hex(pal().acc));
        painter.galley(pos2(p.x + pad.x, y + g_head.size().y + 4.0 * s), g_body, hex(pal().tip_text));
    }

    /// Value entry: Enter or clicking elsewhere confirms, Esc cancels.
    fn draw_edit(&mut self, ui: &mut egui::Ui, st: &Ctx, origin: Pos2, s: f32) {
        let Some(mut e) = self.edit.take() else { return };
        let r = Rect::from_min_size(pos2(origin.x + e.rect[0] * s, origin.y + e.rect[1] * s), vec2(e.rect[2] * s, e.rect[3] * s));
        ui.painter().rect_filled(r, 0.0, hex(pal().plot));
        ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0_f32, hex(pal().rule)), StrokeKind::Inside);
        let id = egui::Id::new("mbc4-edit");
        let family = if self.light { FontFamily::Name("light".into()) } else { FontFamily::Monospace };
        let resp = ui.put(
            r.shrink2(vec2(2.0 * s, 0.0)),
            egui::TextEdit::singleline(&mut e.text)
                .id(id)
                .frame(egui::Frame::NONE)
                .horizontal_align(e.align)
                .char_limit(10)
                .font(FontId::new(VAL.size() * s, family))
                .text_color(hex(pal().ink)),
        );
        if e.fresh {
            resp.request_focus();
            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
                let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(e.text.chars().count()));
                state.cursor.set_char_range(Some(all));
                state.store(ui.ctx(), id);
            }
            e.fresh = false;
            self.edit = Some(e);
            return;
        }
        let (enter, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
        if esc {
            return;
        }
        if enter || resp.lost_focus() {
            self.apply_edit(st, &e);
        } else {
            self.edit = Some(e);
        }
    }

    /// Returns whether a Japanese face was found.
    fn setup_fonts(ctx: &egui::Context) -> bool {
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert("chivo".into(), Arc::new(egui::FontData::from_static(CHIVO)));
        fonts.font_data.insert("chivo-light".into(), Arc::new(egui::FontData::from_static(CHIVO_LIGHT)));
        let mut chain = vec!["chivo".to_owned()];
        // Japanese tooltip text: the OS gothic face, loaded at run time
        for path in [
            r"C:\Windows\Fonts\YuGothM.ttc",
            r"C:\Windows\Fonts\meiryo.ttc",
            "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
        ] {
            if let Ok(bytes) = std::fs::read(path) {
                fonts.font_data.insert("jp".into(), Arc::new(egui::FontData::from_owned(bytes)));
                chain.push("jp".into());
                break;
            }
        }
        for fam in [FontFamily::Proportional, FontFamily::Monospace] {
            let list = fonts.families.entry(fam).or_default();
            for (n, name) in chain.iter().enumerate() {
                list.insert(n, name.clone());
            }
        }
        let mut light = vec!["chivo-light".to_owned()];
        light.extend(chain.iter().skip(1).cloned());
        fonts.families.insert(FontFamily::Name("light".into()), light);
        ctx.set_fonts(fonts);
        chain.len() > 1
    }
}

impl EditorUi<Mbc4Params> for Mbc4Ui {
    fn opened(&mut self, _st: &Ctx) {
        self.asked_size = false;
        self.theme_check = None;
    }

    fn ui(&mut self, ui: &mut egui::Ui, st: &Ctx) {
        let ctx = ui.ctx().clone();
        // Once per open: if the host kept a different window, ask for the size MBC4 wants.
        if !self.asked_size {
            self.asked_size = true;
            let (w, h) = wanted_size(&self.params);
            let r = ui.max_rect();
            if (r.width() - w as f32).abs() > 1.0 || (r.height() - h as f32).abs() > 1.0 {
                let _ = st.request_resize(w, h);
            }
        }
        // A fresh host context only carries Chivo Mono: re-add the Japanese fallback.
        if !self.fonts_ready || (self.jp_font && !ctx.fonts_mut(|f| f.has_glyphs(&FontId::new(10.0, FontFamily::Proportional), "圧縮"))) {
            self.jp_font = Self::setup_fonts(&ctx);
            self.fonts_ready = true;
        }
        ctx.request_repaint();
        // the theme is one setting for the whole family: pick up changes made in other plugins
        if self.theme_check.is_none_or(|t| t.elapsed().as_secs_f32() >= 1.0) {
            self.theme_check = Some(Instant::now());
            let m = settings_mtime();
            if m != self.theme_mtime {
                self.theme_mtime = m;
                load_theme();
            }
        }
        // follow the theme with the glyph coverage (re-rasterises the font atlas only on a change)
        let want = text_alpha();
        if ctx.global_style().visuals.text_options.alpha_from_coverage != want {
            ctx.all_styles_mut(|s| s.visuals.text_options.alpha_from_coverage = want);
        }
        let full = ui.max_rect();
        let s = (full.width() / self.w()).min(full.height() / H);
        let origin = full.min;
        // remember the scale (as the folded width), but only scales the user can pick (75 – 125 %),
        // and not while the host is still catching up with a panel toggle
        let w = (W * s).round() as u32;
        let (lo, hi) = SCALE_RANGE;
        let settled = (full.width() / full.height() - self.w() / H).abs() < 0.01;
        if settled && s >= lo - 0.001 && s <= hi + 0.001 && self.params.ui_width.load() != w {
            self.params.ui_width.store(w);
        }

        // own the whole surface so egui widgets underneath never see the clicks
        let _ = ui.allocate_rect(full, egui::Sense::click_and_drag());
        ui.painter().rect_filled(full, 0.0, hex(pal().cell));

        // display ticks at 30 Hz, whatever the frame rate
        let now = Instant::now();
        let dt = self.tick_t.map_or(1.0 / TICK_HZ, |t| (now - t).as_secs_f32()).min(0.5);
        self.tick_t = Some(now);
        self.tick_acc += dt;
        let mut ticks = 0;
        while self.tick_acc >= 1.0 / TICK_HZ && ticks < 4 {
            self.tick_acc -= 1.0 / TICK_HZ;
            self.tick(st);
            ticks += 1;
        }
        self.tick_acc = self.tick_acc.min(1.0 / TICK_HZ);

        let ppp = ctx.pixels_per_point();
        let painter = ui.painter().clone();
        self.light = ctx.fonts(|f| f.families().contains(&FontFamily::Name("light".into())));
        let pen = Pen { p: &painter, o: origin, s, ppp, k: s * ppp, light: self.light };
        if self.edit.is_none() {
            self.input(&ctx, &pen, st);
        }
        self.commit(st);
        self.draw(&pen, st);
        // the resize corner: a staircase of cells in the window's corner
        for r in 0..3 {
            for c in (2 - r)..3 {
                pen.rect(self.w() - 13.0 + 4.0 * c as f32, H - 13.0 + 4.0 * r as f32, 3.0, 3.0, hex(pal().ink3));
            }
        }
        ui.visuals_mut().selection.bg_fill = hex(pal().acc);
        ui.visuals_mut().selection.stroke = Stroke::new(1.0_f32, hex(pal().plot));
        ui.visuals_mut().text_cursor.stroke = Stroke::new(1.0_f32, hex(pal().ink));
        self.draw_edit(ui, st, origin, s);
        self.draw_tooltip(&ctx, origin, s, full);
    }
}

/// Glyph coverage → alpha for the theme: a little heavier than linear so the small Light
/// sizes stay crisp (dark ink on light), egui's sharp curve for light ink on dark.
fn text_alpha() -> egui::epaint::AlphaFromCoverage {
    if DARK.load(Relaxed) { egui::epaint::AlphaFromCoverage::TwoCoverageMinusCoverageSq } else { egui::epaint::AlphaFromCoverage::Gamma(0.7) }
}

fn visuals() -> egui::Visuals {
    let mut v = egui::Visuals::light();
    v.text_options.alpha_from_coverage = text_alpha();
    v
}

/// The size MBC4 wants: the saved scale (within 75 – 125 %, 100 % the first time) of the
/// current layout, folded or with the INPUT / OUTPUT panel.
fn wanted_size(params: &Mbc4Params) -> (u32, u32) {
    let stored = params.ui_width.load();
    let (lo, hi) = SCALE_RANGE;
    let s = if stored == 0 { DEFAULT_SCALE } else { (stored as f32 / W).clamp(lo, hi) };
    ((width_of(params.io_open.load() != 0) * s).round() as u32, (H * s).round() as u32)
}

pub fn editor(params: Arc<Mbc4Params>) -> Box<dyn Editor> {
    let (w, h) = wanted_size(&params);
    let (_, hi) = SCALE_RANGE;
    let lo = MIN_FIT_SCALE;
    EguiEditor::with_ui(params.clone(), (w, h), Mbc4Ui::new(params))
        // Chivo Mono from the first frame; the Japanese fallback joins on frame one
        .with_font(CHIVO)
        .with_visuals(visuals())
        .resizable(true)
        // no aspect lock: the panel changes the ratio; the UI scales to fit whatever the host gives
        .aspect_ratio(None)
        .min_size(((W * lo) as u32, (H * lo) as u32))
        .max_size((((W + IO_W) * hi) as u32, (H * hi) as u32))
        .into_editor()
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, RawInput};

    fn frame(ctx: &egui::Context, ui: &mut Mbc4Ui, st: &PluginContext<Mbc4Params>, events: Vec<Event>) -> egui::FullOutput {
        let mut ri = RawInput::default();
        ri.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(W * 1.5, H * 1.5)));
        ri.events = events;
        ctx.run_ui(ri, |u| ui.ui(u, st))
    }

    fn texts(out: &egui::FullOutput) -> Vec<String> {
        out.shapes
            .iter()
            .filter_map(|c| if let egui::Shape::Text(t) = &c.shape { Some(t.galley.text().to_owned()) } else { None })
            .collect()
    }

    fn setup() -> (Arc<Mbc4Params>, PluginContext<Mbc4Params>, Mbc4Ui, egui::Context) {
        let params = Arc::new(Mbc4Params::new());
        let st = truce_core::editor::for_test_params(params.clone()).with_params(params.clone());
        let ui = Mbc4Ui::new(params.clone());
        (params, st, ui, egui::Context::default())
    }

    #[test]
    fn paints_every_section() {
        let (params, st, mut ui, ctx) = setup();
        // folded by default: no INPUT / OUTPUT panel
        let mut out = None;
        for _ in 0..3 {
            out = Some(frame(&ctx, &mut ui, &st, vec![]));
        }
        let t = texts(&out.unwrap());
        for s in ["I/O", "A", "D", "GAIN", "RAT", "MATCH", "0.0", "0 dB", "2:1", "25 ms"] {
            assert!(t.iter().any(|x| x == s), "{s} missing: {t:?}");
        }
        assert!(t.iter().any(|x| x == "120\u{2013}1.00k Hz"), "band B range missing: {t:?}");
        assert!(!t.iter().any(|x| x == "INPUT" || x == "LOOK"), "panel drawn while folded: {t:?}");

        params.io_open.store(1);
        let mut out = None;
        for _ in 0..3 {
            let mut ri = RawInput::default();
            ri.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(width_of(true) * 1.5, H * 1.5)));
            out = Some(ctx.run_ui(ri, |u| ui.ui(u, &st)));
        }
        let t = texts(&out.unwrap());
        for s in ["INPUT", "OUTPUT", "LOOK", "MIX"] {
            assert!(t.iter().any(|x| x == s), "{s} missing with the panel open: {t:?}");
        }
    }

    #[test]
    fn tooltip_paints_heading_and_japanese_body() {
        let (_, st, mut ui, ctx) = setup();
        // hover band A's THS fader body
        // hover band A's RAT slider
        let p = pos2((strip_x(0) + 100.0) * 1.5, SLIDER_Y0 * 1.5);
        for _ in 0..3 {
            frame(&ctx, &mut ui, &st, vec![Event::PointerMoved(p)]);
        }
        std::thread::sleep(std::time::Duration::from_millis(600));
        let out = frame(&ctx, &mut ui, &st, vec![Event::PointerMoved(p)]);
        let t = texts(&out);
        assert!(t.iter().any(|s| s == "RATIO"), "tooltip heading missing: {t:?}");
        let body = t.iter().find(|s| s.contains("圧縮の比率")).expect("tooltip body missing");
        let missing = ctx.fonts_mut(|f| !f.has_glyphs(&FontId::new(15.75, FontFamily::Proportional), body));
        assert!(!missing, "Japanese glyphs are not in any loaded font");
    }

    /// like `for_test_params`, but parameter writes reach the params
    fn writing_ctx(params: &Arc<Mbc4Params>) -> PluginContext<Mbc4Params> {
        use truce_core::editor::ClosureBridge;
        let (w, g, gp) = (params.clone(), params.clone(), params.clone());
        PluginContext::from_closures(
            ClosureBridge {
                begin_edit: Box::new(|_| {}),
                set_param: Box::new(move |id, v| w.set_normalized(id, v)),
                end_edit: Box::new(|_| {}),
                request_resize: Box::new(|_, _| false),
                get_param: Box::new(move |id| g.get_normalized(id).unwrap_or(0.0)),
                get_param_plain: Box::new(move |id| gp.get_plain(id).unwrap_or(0.0)),
                format_param: Box::new(|_| String::new()),
                get_meter: Box::new(|_| 0.0),
                get_state: Box::new(Vec::new),
                set_state: Box::new(|_| {}),
                transport: Box::new(|| None),
            },
            params.clone(),
        )
        .with_params(params.clone())
    }

    #[test]
    fn dragging_in_the_graph_moves_the_band_threshold() {
        let (params, _, mut ui, ctx) = setup();
        let st = writing_ctx(&params);
        // inside band B (between 120 Hz and 1 kHz), from 0 dB down by 20 dB
        let x = freq_to_x(400.0) * 1.5;
        let y0 = db_to_y(0.0) * 1.5;
        let y1 = db_to_y(-20.0) * 1.5;
        let press = |p: Pos2, pressed| Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&ctx, &mut ui, &st, vec![Event::PointerMoved(pos2(x, y0))]);
        frame(&ctx, &mut ui, &st, vec![press(pos2(x, y0), true)]);
        frame(&ctx, &mut ui, &st, vec![Event::PointerMoved(pos2(x, y1))]);
        frame(&ctx, &mut ui, &st, vec![press(pos2(x, y1), false)]);
        assert!((params.b_thresh.value() + 20.0).abs() < 0.01, "B threshold {}", params.b_thresh.value());
        assert!(params.a_thresh.value().abs() < 0.01);
    }

    #[test]
    fn small_host_window_fits_instead_of_clipping() {
        let (_, st, mut ui, ctx) = setup();
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(900.0, 600.0));
        let mut out = None;
        for _ in 0..3 {
            let mut ri = RawInput::default();
            ri.screen_rect = Some(screen);
            out = Some(ctx.run_ui(ri, |u| ui.ui(u, &st)));
        }
        let mut bb = Rect::NOTHING;
        for c in &out.unwrap().shapes {
            bb = bb.union(c.shape.visual_bounding_rect());
        }
        assert!(bb.max.x <= screen.max.x + 0.5 && bb.max.y <= screen.max.y + 0.5, "painted outside the window: {bb:?}");
    }
}
