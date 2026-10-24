//! MBC4 editor: one painted surface on egui (the JUCE 1.1 PC1-style UI, same layout).
//!
//! Everything is laid out in design units (1240 × 800) and scaled uniformly.
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
use std::sync::Arc;
use std::time::Instant;
use truce::prelude::*;
use truce_core::editor::PluginContext;
use truce_egui::{EditorUi, EguiEditor};
use truce_params::ParamRange;

type Ctx = PluginContext<Mbc4Params>;

// ───────────────────────────────── tokens

const W: f32 = 1240.0;
const H: f32 = 800.0;
const DEFAULT_SCALE: f32 = 1.5;
/// 100 – 180 % (the corner drag and the size MBC4 asks the host for)
const SCALE_RANGE: (f32, f32) = (1.0, 1.8);
/// Hard floor given to the host: a host that keeps a smaller window gets the
/// whole UI scaled down to fit instead of a clipped one.
const MIN_FIT_SCALE: f32 = 0.5;

fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// PC1 style: one background colour, 0.2 px rules, near-black ink in three steps,
/// square cells for every mark, signal orange as the only accent (on / active / cut).
struct Pal;
impl Pal {
    /// the one background
    const CELL: u32 = 0xE8E9E5;
    const RULE: u32 = 0xC3C4BF;
    /// text, fader caps, reference lines
    const INK: u32 = 0x111210;
    /// secondary text, scale numbers
    const INK2: u32 = 0x575854;
    /// notes, off toggles
    const INK3: u32 = 0x8F908B;
    /// unlit cells
    const OFF: u32 = 0xDCDDD8;
    /// lit cells (fader amounts)
    const LIT: u32 = 0xB3B4AF;
    /// selected segment, text on dark
    const PLOT: u32 = 0xF4F4F1;
    /// signal orange
    const ACC: u32 = 0xFF4D0F;
    /// input spectrum behind the output
    const GHOST: u32 = 0xD3D4CF;
    // disabled (bypassed band)
    const D_LIT: u32 = 0xCBCCC7;
    const D_KNOB: u32 = 0xA8A9A4;
    const D_TEXT: u32 = 0xB0B1AC;
    /// data intensity, weak → strong
    const RAMP: [u32; 6] = [0xC7C8C3, 0xB9BAB5, 0xA6A7A2, 0x8F908B, 0x5C5D59, 0x2A2B29];
}

/// ramp index for row j of n (darker towards the top)
fn ramp_index(j: usize, n: usize) -> usize {
    ((j as f32 / n as f32 * Pal::RAMP.len() as f32 * 1.15) as usize).min(Pal::RAMP.len() - 1)
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
}
use Role::{Label as LABEL, Note as NOTE, Tick as TICK, Value as VAL};

impl Role {
    fn size(self) -> f32 {
        match self {
            Role::Label => 8.0,
            Role::Value => 9.0,
            Role::Note | Role::Tick => 7.5,
        }
    }
    fn tracking(self) -> f32 {
        match self {
            Role::Label => 0.8,
            Role::Note => 0.6,
            Role::Value | Role::Tick => 0.0,
        }
    }
    fn light(self) -> bool {
        self != Role::Label
    }
}

// ───────────────────────────────── layout grid (10 px module; cells 3 × 3 on a 4 px pitch)

const FRAME_L: f32 = 20.0;
const FRAME_T: f32 = 20.0;
const FRAME_R: f32 = 1220.0;
const FRAME_B: f32 = 760.0;
/// input | main | output
const COL_L: f32 = 140.0;
const COL_R: f32 = 1100.0;
/// logo cell bottom (input column)
const LOGO_B: f32 = 50.0;
/// graph | band strips
const ROW_B: f32 = 380.0;
const STRIP_W: f32 = 240.0;
const PITCH: f32 = 4.0;
const CELL: f32 = 3.0;
// graph plot: +6 .. −72 dB
const PLOT_L: f32 = 150.0;
const PLOT_R: f32 = 1050.0;
const PLOT_T: f32 = 80.0;
const PLOT_B: f32 = 340.0;
const MAX_DB: f32 = 6.0;
const MIN_DB: f32 = -72.0;
/// meter peak readouts / frequency labels
const READOUT_TOP: f32 = 352.0;
// vertical faders: label row, 40 cells, value row
const FADER_TOP: f32 = 418.0;
const FADER_H: f32 = 202.0;
const FADER_LABEL_Y: f32 = 425.0;
const FADER_CELLS_Y: f32 = 440.0;
const FADER_VALUE_Y: f32 = 612.0;
const FADER_ROWS: usize = 40;
/// clickable width (they paint 44 wide)
const FADER_GRAB: f32 = 28.0;
// horizontal sliders: centre of the first row, from the strip's left edge
const SLIDER_Y0: f32 = 650.0;
const SLIDER_PITCH: f32 = 20.0;
const SLIDER_CELLS: usize = 32;
// band strip x offsets
const THS_X: f32 = 30.0;
const RNG_X: f32 = 75.0;
const GAIN_X: f32 = 120.0;
const GR_X: f32 = 210.0;
const HEADER_Y: f32 = 395.0;
const RANGE_X: f32 = 70.0;

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
    Thresh,
    Range,
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
    Analyzer,
}

impl Tip {
    fn text(self) -> (&'static str, &'static str) {
        match self {
            Tip::Thresh => ("Threshold", "この値を超えた分を圧縮します。グラフの中を上下にドラッグしても変えられます。"),
            Tip::Range => ("Range", "ゲインリダクションの上限です。"),
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
            Tip::Analyzer => ("Analyzer", "スペクトラムの表示。IN+OUT / OUT / IN / OFF。"),
        }
    }
}

// band parameter slots
const THR: usize = 0;
const RAT: usize = 1;
const ATK: usize = 2;
const RLS: usize = 3;
const KNE: usize = 4;
const RNG: usize = 5;
const GAIN: usize = 6;
const MIX: usize = 7;
const SOLO: usize = 8;
const BYP: usize = 10;

#[rustfmt::skip]
const BAND_IDS: [[P; 11]; NUM_BANDS] = [
    [P::AThresh, P::ARatio, P::AAttack, P::ARelease, P::AKnee, P::ARange, P::AGain, P::AMix, P::ASolo, P::AMute, P::ABypass],
    [P::BThresh, P::BRatio, P::BAttack, P::BRelease, P::BKnee, P::BRange, P::BGain, P::BMix, P::BSolo, P::BMute, P::BBypass],
    [P::CThresh, P::CRatio, P::CAttack, P::CRelease, P::CKnee, P::CRange, P::CGain, P::CMix, P::CSolo, P::CMute, P::CBypass],
    [P::DThresh, P::DRatio, P::DAttack, P::DRelease, P::DKnee, P::DRange, P::DGain, P::DMix, P::DSolo, P::DMute, P::DBypass],
];
const XO_IDS: [P; 3] = [P::Xover1, P::Xover2, P::Xover3];

fn bid(b: usize, slot: usize) -> u32 {
    BAND_IDS[b][slot].into()
}

// ───────────────────────────────── controls

/// Fader drawn with cells: the cells from the base value to the current value are lit,
/// the value's own cell is black with a thin needle through it.
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
}

impl Fader {
    fn cells(&self) -> usize {
        if self.vertical { FADER_ROWS } else { SLIDER_CELLS }
    }
    fn rect(&self) -> [f32; 4] {
        if self.vertical { [self.x - FADER_GRAB / 2.0, self.y, FADER_GRAB, FADER_H] } else { [self.x, self.y - 7.0, 228.0, 14.0] }
    }
    fn value_rect(&self) -> [f32; 4] {
        if self.vertical { [self.x - 22.0, FADER_VALUE_Y - 7.0, 44.0, 14.0] } else { [self.x + 174.0, self.y - 7.0, 50.0, 14.0] }
    }
}

fn faders() -> Vec<Fader> {
    let mut v = Vec::new();
    for b in 0..NUM_BANDS {
        let x0 = strip_x(b);
        for (slot, cx, label, tip) in [(THR, THS_X, "THS", Tip::Thresh), (RNG, RNG_X, "RNG", Tip::Range), (GAIN, GAIN_X, "GAIN", Tip::Gain)] {
            v.push(Fader { id: bid(b, slot), vertical: true, x: x0 + cx, y: FADER_TOP, band: Some(b), label, steps: DB_STEPS, format: Format::Db, tip });
        }
        for (j, (slot, label, steps, format, tip)) in [
            (RAT, "RAT", RATIO_STEPS, Format::Ratio, Tip::Ratio),
            (KNE, "KNE", DB_STEPS, Format::Db, Tip::Knee),
            (ATK, "ATK", ATK_STEPS, Format::Ms, Tip::Attack),
            (RLS, "RLS", RLS_STEPS, Format::Ms, Tip::Release),
            (MIX, "MIX", MIX_STEPS, Format::Percent, Tip::BandMix),
        ]
        .into_iter()
        .enumerate()
        {
            let y = SLIDER_Y0 + SLIDER_PITCH * j as f32;
            v.push(Fader { id: bid(b, slot), vertical: false, x: x0 + 6.0, y, band: Some(b), label, steps, format, tip });
        }
    }
    for (id, cx, label, steps, format, tip) in [
        (P::InGain, 80.0, "IN", DB_STEPS, Format::Db, Tip::Input),
        (P::Lookahead, 1124.0, "LOOK", LOOK_STEPS, Format::Ms, Tip::Look),
        (P::Mix, 1160.0, "MIX", MIX_STEPS, Format::Percent, Tip::Mix),
        (P::OutGain, 1196.0, "OUT", DB_STEPS, Format::Db, Tip::Output),
    ] {
        v.push(Fader { id: id.into(), vertical: true, x: cx, y: FADER_TOP, band: None, label, steps, format, tip });
    }
    v
}

fn toggle_rect(b: usize, j: usize) -> [f32; 4] {
    [strip_x(b) + 166.0 + 20.0 * j as f32, ROW_B + 7.0, 18.0, 16.0]
}
fn match_rect(b: usize) -> [f32; 4] {
    [strip_x(b) + 144.0, FADER_CELLS_Y + 4.0 * FADER_ROWS as f32 - 16.0, 42.0, 16.0]
}
const MODES: [&str; 4] = ["IN+OUT", "OUT", "IN", "OFF"];

#[derive(Clone, Copy, PartialEq)]
enum Hit {
    Fader(usize),
    Toggle(usize, usize),
    Match(usize),
    Mode(usize),
    Xo(usize),
    Thresh(usize),
    Plot,
    /// band header: low / high edge frequency text
    BandLo(usize),
    BandHi(usize),
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
    /// one-device-pixel frame of a snapped rectangle
    fn outline(&self, x: f32, y: f32, w: f32, h: f32, col: Color32) {
        let (x0, y0) = ((x * self.k).round(), (y * self.k).round());
        let x1 = ((x + w) * self.k).round().max(x0 + 1.0);
        let y1 = ((y + h) * self.k).round().max(y0 + 1.0);
        for (a, b, c, d) in [(x0, y0, x1, y0 + 1.0), (x0, y1 - 1.0, x1, y1), (x0, y0, x0 + 1.0, y1), (x1 - 1.0, y0, x1, y1)] {
            self.p.rect_filled(Rect::from_min_max(self.px(a, b), self.px(c, d)), 0.0, col);
        }
    }
    /// 0.2 px rules
    fn rule_h(&self, x1: f32, x2: f32, y: f32) {
        self.fill(x1, y - 0.1, x2 - x1, 0.2, hex(Pal::RULE));
    }
    fn rule_v(&self, x: f32, y1: f32, y2: f32) {
        self.fill(x - 0.1, y1, 0.2, y2 - y1, hex(Pal::RULE));
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
        self.p.rect_filled(r, (3.0 * self.s) as u8, hex(Pal::INK));
        self.text(s, x + w / 2.0, cy, VAL, hex(Pal::PLOT), Align2::CENTER_CENTER);
    }
    fn section_header(&self, number: &str, name: &str, x: f32, y: f32, col: Color32) {
        self.text(number, x, y, NOTE, hex(Pal::INK3), Align2::LEFT_CENTER);
        self.text(name, x + 14.0, y, LABEL, col, Align2::LEFT_CENTER);
    }
}

const DIGITS: [[&str; 7]; 10] = [
    ["01110", "10001", "10001", "10001", "10001", "10001", "01110"],
    ["00100", "01100", "00100", "00100", "00100", "00100", "01110"],
    ["01110", "10001", "00001", "00010", "00100", "01000", "11111"],
    ["11111", "00010", "00100", "00010", "00001", "10001", "01110"],
    ["00010", "00110", "01010", "10010", "11111", "00010", "00010"],
    ["11111", "10000", "11110", "00001", "00001", "10001", "01110"],
    ["00110", "01000", "10000", "11110", "10001", "10001", "01110"],
    ["11111", "00001", "00010", "00100", "01000", "01000", "01000"],
    ["01110", "10001", "10001", "01110", "10001", "10001", "01110"],
    ["01110", "10001", "10001", "01111", "00001", "00010", "01100"],
];
const MINUS: [&str; 7] = ["00000", "00000", "00000", "11111", "00000", "00000", "00000"];
const PLUS: [&str; 7] = ["00000", "00100", "00100", "11111", "00100", "00100", "00000"];
const DOT: [&str; 7] = ["0", "0", "0", "0", "0", "0", "1"];

fn digit_glyph(ch: char) -> Option<&'static [&'static str; 7]> {
    match ch {
        '0'..='9' => Some(&DIGITS[ch as usize - '0' as usize]),
        '-' => Some(&MINUS),
        '+' => Some(&PLUS),
        '.' => Some(&DOT),
        _ => None,
    }
}

/// 5 × 7 dot-matrix number, unlit dots drawn faintly
fn dot_number(pen: &Pen<'_>, s: &str, cx: f32, top: f32, col: Color32) {
    // glyph columns (1 empty column between glyphs), on a 2 px pitch
    let mut cols: i32 = -1;
    for ch in s.chars() {
        if let Some(g) = digit_glyph(ch) {
            cols += g[0].len() as i32 + 1;
        }
    }
    if cols <= 0 {
        return;
    }
    let probe = pen.lat(0.0, 0.0, 2.0, 1.6);
    let l = pen.lat(cx - probe.ext(cols as usize) * 0.5, top, 2.0, 1.6);
    let mut col0 = 0;
    for ch in s.chars() {
        let Some(g) = digit_glyph(ch) else { continue };
        let w = g[0].len();
        for (r, row) in g.iter().enumerate() {
            for (c, b) in row.chars().enumerate() {
                pen.cell(&l, col0 + c, r, if b == '1' { col } else { hex(Pal::OFF) });
            }
        }
        col0 += w + 1;
    }
}

/// 4 × 5 dot logo, the last letter in the accent
fn logo(pen: &Pen<'_>, x: f32, y: f32) {
    const LETTERS: [[&str; 5]; 4] = [
        ["1001", "1111", "1111", "1001", "1001"], // M
        ["1110", "1001", "1110", "1001", "1110"], // B
        ["0111", "1000", "1000", "1000", "0111"], // C
        ["1010", "1010", "1111", "0010", "0010"], // 4
    ];
    let l = pen.lat(x, y, PITCH, CELL);
    for (n, g) in LETTERS.iter().enumerate() {
        let c = if n == 3 { hex(Pal::ACC) } else { hex(Pal::INK) };
        for (r, row) in g.iter().enumerate() {
            for (cc, b) in row.chars().enumerate() {
                if b == '1' {
                    pen.cell(&l, n * 5 + cc, r, c);
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

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Both,
    Post,
    Pre,
    Off,
}

impl Mode {
    fn from(i: u32) -> Self {
        match i {
            1 => Mode::Post,
            2 => Mode::Pre,
            3 => Mode::Off,
            _ => Mode::Both,
        }
    }
}

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

    /// The pre / post spectrum: pull new samples, analyse when there are enough.
    fn update(&mut self, sh: &Shared, mode: Mode) {
        let sr = sh.sample_rate.load(std::sync::atomic::Ordering::Relaxed) as f32;
        // the input is also needed in "out" mode: it shows how much each band removes
        self.analyse(&sh.pre, true, mode != Mode::Off, sr);
        self.analyse(&sh.post, false, mode == Mode::Both || mode == Mode::Post, sr);
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
        if s.new_samples < FFT / 4 || sr <= 0.0 {
            // no fresh audio: let the spectrum fall
            for v in &mut s.display {
                *v = (*v - 1.5).max(SILENT_DB);
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
            *v += (db - *v) * if db > *v { 0.5 } else { 0.14 };
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
        self.shown = if v > self.shown { v } else { v.max(self.shown - 1.2) };
        // held value: stays 1.5 s, then falls slowly so it can be read
        if v >= self.hold {
            self.hold = v;
            self.hold_time = 45;
        } else {
            self.hold_time -= 1;
            if self.hold_time < 0 {
                self.hold = v.max(self.hold - 0.6);
            }
        }
    }
}

/// GR meter: 0 .. 24 dB over 40 cells
const GR_METER_RANGE: f32 = 24.0;
/// held GR value: refreshed every 0.5 s
const READOUT_HOLD_FRAMES: i32 = 15;
const LEARN_SECONDS: f64 = 3.0;
/// display ticks per second (the JUCE editor timer)
const TICK_HZ: f32 = 30.0;

#[derive(Clone, Copy, Default)]
struct GrMeter {
    shown: f32,
    window_peak: f32,
    readout: f32,
    frames: i32,
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
    /// segment widths of the analyser switch (measured while drawing)
    mode_w: [f32; 4],
    last_req: Option<(u32, u32)>,
    asked_size: bool,
}

impl Mbc4Ui {
    fn new(params: Arc<Mbc4Params>) -> Self {
        let infos = params.param_infos().into_iter().map(|i| (i.id, (i.range.clone(), i.default_plain))).collect();
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
            mode_w: [54.0, 35.0, 28.0, 35.0],
            last_req: None,
            asked_size: false,
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
    fn set(&self, st: &Ctx, id: u32, plain: f64) {
        st.set_param(id, self.norm(id, plain));
    }
    /// one complete host gesture
    fn set_once(&self, st: &Ctx, id: u32, plain: f64) {
        st.begin_edit(id);
        self.set(st, id, plain);
        st.end_edit(id);
    }
    fn default_of(&self, id: u32) -> f64 {
        self.infos.get(&id).map_or(0.0, |i| i.1)
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
    fn mode_rect(&self, i: usize) -> [f32; 4] {
        let total: f32 = self.mode_w.iter().sum();
        let x0 = COL_R - 10.0 - total.ceil();
        [x0 + self.mode_w[..i].iter().sum::<f32>(), FRAME_T + 7.0, self.mode_w[i], 16.0]
    }

    fn hit(&self, pen: &Pen<'_>, st: &Ctx, p: Pos2) -> Option<Hit> {
        for (i, f) in self.faders.iter().enumerate() {
            if in_r(f.rect(), p) {
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
        for i in 0..MODES.len() {
            if in_r(self.mode_rect(i), p) {
                return Some(Hit::Mode(i));
            }
        }
        if in_r([FRAME_R - 2.0, FRAME_B + 2.0, W - FRAME_R + 2.0, H - FRAME_B - 2.0], p) {
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
            Hit::Mode(_) => Tip::Analyzer,
            // the graph shows bubbles instead
            Hit::Xo(_) | Hit::Thresh(_) | Hit::Plot | Hit::BandLo(_) | Hit::BandHi(_) | Hit::Resize => return None,
        })
    }

    // ── input

    #[allow(clippy::too_many_lines)]
    fn input(&mut self, ctx: &egui::Context, pen: &Pen<'_>, st: &Ctx) {
        let (pos, pressed, down, released, dbl, fine, wheel) = ctx.input(|i| {
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
                m.shift || m.ctrl || m.command || m.mac_cmd,
                wheel,
            )
        });
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
                    let w = (d.w0 + (pos.x - d.start.x).max((pos.y - d.start.y) * W / H)).clamp(W * SCALE_RANGE.0, W * SCALE_RANGE.1);
                    let (rw, rh) = (w.round() as u32, (w * H / W).round() as u32);
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
            Some(Hit::Xo(_)) => egui::CursorIcon::ResizeHorizontal,
            Some(Hit::Thresh(_) | Hit::Plot) => egui::CursorIcon::ResizeVertical,
            Some(Hit::Resize) => egui::CursorIcon::ResizeNwSe,
            Some(Hit::Mode(_)) => egui::CursorIcon::PointingHand,
            _ => egui::CursorIcon::Default,
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
                        self.set_once(st, f.id, self.default_of(f.id));
                    }
                }
                Hit::Xo(i) => self.set_once(st, XO_IDS[i].into(), self.default_of(XO_IDS[i].into())),
                Hit::Thresh(b) => self.set_once(st, bid(b, THR), self.default_of(bid(b, THR))),
                Hit::Plot => {
                    let b = self.band_at(st, p.x);
                    self.set_once(st, bid(b, THR), self.default_of(bid(b, THR)));
                }
                Hit::BandLo(b) => self.open_xo_edit(st, b, b - 1, self.lo_text_rect(pen, st, b)),
                Hit::BandHi(b) => self.open_xo_edit(st, b, b, self.hi_text_rect(pen, st, b)),
                _ => {}
            }
            return;
        }

        if pressed && let Some(h) = h {
            self.hover = None;
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
                Hit::Mode(i) => self.params.analyzer.store(i as u32),
                Hit::Resize => {
                    self.last_req = None;
                    self.drag = Some(Drag { hit: h, id: u32::MAX, anchor: p, anchor_norm: 0.0, fine: false, raw: 0.0, last_y: p.y, start: pos, w0: W * s });
                }
                Hit::BandLo(_) | Hit::BandHi(_) => {}
            }
            return;
        }

        if let Some(Hit::Fader(i)) = h {
            for d in wheel {
                self.wheel(st, i, if d > 0.0 { 1 } else { -1 }, fine);
            }
        }
    }

    fn wheel(&self, st: &Ctx, i: usize, dir: i32, fine: bool) {
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

    fn apply_edit(&self, st: &Ctx, e: &Edit) {
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

    fn update_learn(&self, st: &Ctx, b: usize) {
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
        self.an.update(&sh, Mode::from(self.params.analyzer.load()));
        for b in 0..NUM_BANDS {
            let gr = sh.gr_db(b);
            let d = &mut self.gr_disp[b];
            *d = if gr < *d { gr } else { *d + (gr - *d) * 0.25 };

            let byp = self.bypassed(st, b);
            let m = &mut self.gr_meters[b];
            let g = if byp { 0.0 } else { (-gr).min(GR_METER_RANGE) }; // positive dB
            m.shown = if g > m.shown { g } else { m.shown + (g - m.shown) * 0.25 };
            // held value: the peak of each half second, so it can be read
            m.window_peak = m.window_peak.max(g);
            m.frames += 1;
            if m.frames >= READOUT_HOLD_FRAMES {
                m.readout = m.window_peak;
                m.window_peak = 0.0;
                m.frames = 0;
            }
            self.update_learn(st, b);
        }
        self.in_meter.update(sh.in_peak_db());
        self.out_meter.update(sh.out_peak_db());
    }

    // ── drawing

    fn draw(&mut self, pen: &Pen<'_>, st: &Ctx) {
        // blocks are separated by rules only
        pen.rule_h(FRAME_L, FRAME_R, FRAME_T);
        pen.rule_h(FRAME_L, FRAME_R, FRAME_B);
        pen.rule_v(FRAME_L, FRAME_T, FRAME_B);
        pen.rule_v(FRAME_R, FRAME_T, FRAME_B);
        pen.rule_v(COL_L, FRAME_T, FRAME_B);
        pen.rule_v(COL_R, FRAME_T, FRAME_B);
        pen.rule_h(FRAME_L, COL_L, LOGO_B);
        pen.rule_h(COL_L, COL_R, ROW_B);
        for i in 1..NUM_BANDS {
            pen.rule_v(strip_x(i), ROW_B, FRAME_B);
        }

        logo(pen, 30.0, 25.5);
        pen.section_header("01", "INPUT", 30.0, 65.0, hex(Pal::INK));
        pen.section_header("07", "OUTPUT", COL_R + 10.0, 35.0, hex(Pal::INK));
        self.draw_meter(pen, 80.0, self.in_meter);
        self.draw_meter(pen, 1160.0, self.out_meter);
        pen.text(concat!("V", env!("CARGO_PKG_VERSION")), FRAME_L, 771.0, NOTE, hex(Pal::INK3), Align2::LEFT_CENTER);

        self.draw_graph(pen, st);
        for b in 0..NUM_BANDS {
            self.draw_strip(pen, st, b);
        }
        for i in 0..self.faders.len() {
            self.draw_fader(pen, st, i);
        }
        self.draw_bubbles(pen, st);
    }

    /// two columns of dots on the graph's rows (same lattice); above 0 dB in orange
    fn draw_meter(&self, pen: &Pen<'_>, cx: f32, m: Meter) {
        let l = pen.lat(cx - 3.5, PLOT_T, PITCH, CELL);
        let rows = l.fit(PLOT_B - PLOT_T);
        let (n, r0) = (rows_for(m.shown, rows), rows_for(0.0, rows));
        for j in 0..rows {
            let c = if j >= n {
                hex(Pal::OFF)
            } else if j >= r0 {
                hex(Pal::ACC)
            } else {
                hex(Pal::RAMP[ramp_index(j, rows)])
            };
            pen.cell(&l, 0, rows - 1 - j, c);
            pen.cell(&l, 1, rows - 1 - j, c);
        }
        // 0 dB reference: exactly as wide as the dots
        pen.line_h(l.x(0), l.x(0) + l.ext(2), db_to_y(0.0), hex(Pal::INK), 0.5);
        let readout = if m.hold <= MIN_DB {
            "--.-".to_owned()
        } else {
            format!("{}{:.1}", if m.hold > 0.0 { "+" } else { "" }, m.hold)
        };
        dot_number(pen, &readout, cx, READOUT_TOP, if m.hold > 0.0 { hex(Pal::ACC) } else { hex(Pal::INK) });
    }

    /// "02 SPLIT": the spectrum drawn with cells, crossovers and per-band thresholds.
    /// Output spectrum in grey steps, the part each band's compressor removes in orange,
    /// the input behind it in a pale grey.
    fn draw_graph(&mut self, pen: &Pen<'_>, st: &Ctx) {
        pen.section_header("02", "SPLIT", COL_L + 10.0, 35.0, hex(Pal::INK));
        let mode = Mode::from(self.params.analyzer.load());
        self.draw_cells(pen, st, mode);

        // scale: numbers only
        let mut db = 0.0;
        while db >= MIN_DB {
            pen.text(&format!("{db:.0}"), PLOT_R + 7.0, db_to_y(db), TICK, hex(Pal::INK2), Align2::LEFT_CENTER);
            db -= 12.0;
        }
        for (f, t) in [(50.0, "50"), (100.0, "100"), (200.0, "200"), (500.0, "500"), (1000.0, "1k"), (2000.0, "2k"), (5000.0, "5k"), (10000.0, "10k")] {
            pen.text(t, freq_to_x(f), READOUT_TOP, TICK, hex(Pal::INK2), Align2::CENTER_CENTER);
        }

        let hot = self.drag.as_ref().map(|d| d.hit).or(self.hot);
        for i in 0..3 {
            let t = if hot == Some(Hit::Xo(i)) { 1.2 } else { 0.5 };
            pen.line_v(freq_to_x(self.xo(st, i)), PLOT_T, PLOT_B, hex(Pal::INK), t);
        }
        for b in 0..NUM_BANDS {
            let (x0, x1) = (self.band_left_x(st, b), self.band_right_x(st, b));
            let byp = self.bypassed(st, b);
            let thr = self.plain(st, bid(b, THR)) as f32;
            let t = if hot == Some(Hit::Thresh(b)) { 1.2 } else { 0.5 };
            pen.line_h(x0, x1, db_to_y(thr), if byp { hex(Pal::D_KNOB) } else { hex(Pal::INK) }, t);
            pen.text(LETTERS[b], x0 + 3.0, PLOT_T - 10.0, LABEL, if byp { hex(Pal::D_TEXT) } else { hex(Pal::INK2) }, Align2::LEFT_CENTER);
        }

        // analyser switch: a rounded groove, the selected segment raised and framed
        for (i, name) in MODES.iter().enumerate() {
            // JUCE measured the string with its trailing tracking
            self.mode_w[i] = (pen.text_w(name, LABEL) + LABEL.tracking()).round() + 16.0;
        }
        let sel = self.params.analyzer.load().min(3) as usize;
        let r0 = self.mode_rect(0);
        let total: f32 = self.mode_w.iter().sum();
        pen.pill(r0[0], r0[1], total, r0[3], hex(Pal::OFF), None);
        for (i, name) in MODES.iter().enumerate() {
            let r = self.mode_rect(i);
            if i == sel {
                pen.pill(r[0] + 1.0, r[1] + 1.0, r[2] - 2.0, r[3] - 2.0, hex(Pal::PLOT), Some(hex(Pal::RULE)));
            }
            let c = if i == sel { hex(Pal::INK) } else { hex(Pal::INK2) };
            pen.text(name, r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, LABEL, c, Align2::CENTER_CENTER);
        }
    }

    fn draw_cells(&self, pen: &Pen<'_>, st: &Ctx, mode: Mode) {
        let show_out = mode == Mode::Both || mode == Mode::Post;
        let show_in = mode != Mode::Off;
        // whole-pixel lattice: how many cells fit depends a little on the UI scale
        let l = pen.lat(PLOT_L, PLOT_T, PITCH, CELL);
        let (cols, rows) = (l.fit(PLOT_R - PLOT_L), l.fit(PLOT_B - PLOT_T));
        let n_ramp = Pal::RAMP.len();
        let (acc_i, ghost_i, off_i) = (n_ramp, n_ramp + 1, n_ramp + 2);
        let colour = |i: usize| match i {
            i if i < n_ramp => hex(Pal::RAMP[i]),
            i if i == acc_i => hex(Pal::ACC),
            i if i == ghost_i => hex(Pal::GHOST),
            _ => hex(Pal::OFF),
        };
        let byp: [bool; NUM_BANDS] = std::array::from_fn(|b| self.bypassed(st, b));

        // runs of one colour are filled as single rectangles, the gaps drawn over them afterwards
        for c in 0..cols {
            let cx = l.x(c) + l.cell() * 0.5;
            let f = x_to_freq(cx);
            let idx = ((f / MIN_HZ).ln() / (MAX_HZ / MIN_HZ).ln()) as f32 * NUM_COLS as f32 - 0.5;
            let b = self.band_at(st, cx);
            let in_db = self.an.pre.at(idx);
            let gr = if byp[b] { 0.0 } else { self.gr_disp[b] };
            let n_in = if show_in { rows_for(in_db, rows) } else { 0 };
            let n_out = if show_out { rows_for(self.an.post.at(idx), rows) } else { 0 };
            let n_cut = rows_for(in_db + gr, rows);
            let colour_of = |j: usize| {
                if mode == Mode::Pre {
                    return if j < n_in { ramp_index(j, rows) } else { off_i };
                }
                if j < n_out {
                    ramp_index(j, rows)
                } else if j < n_in && j >= n_cut {
                    acc_i // removed by the compressor
                } else if j < n_in && show_in && mode == Mode::Both {
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
                pen.span(&l, c, rows - j, 1, j - run_start, colour(run_c));
                run_start = j;
                run_c = cj;
            }
        }
        // gaps between the cells of a run, in the background colour
        let bg = hex(Pal::CELL);
        let (top, height, left, width) = (l.y(0), l.ext(rows), l.x(0), l.ext(cols));
        for r in 0..rows.saturating_sub(1) {
            pen.fill(left, l.y(r) + l.cell(), width, l.gap(), bg);
        }
        for c in 0..cols.saturating_sub(1) {
            pen.fill(l.x(c) + l.cell(), top, l.gap(), height, bg);
        }
    }

    /// One band: header (number, name, range, S / M / B), the gain match button and the
    /// gain-reduction meter (the faders are drawn with the others).
    fn draw_strip(&self, pen: &Pen<'_>, st: &Ctx, b: usize) {
        let x0 = strip_x(b);
        let dim = self.bypassed(st, b);
        let ink = if dim { hex(Pal::D_TEXT) } else { hex(Pal::INK) };
        pen.section_header(&format!("0{}", b + 3), &format!("BAND {}", LETTERS[b]), x0 + 10.0, HEADER_Y, ink);
        if self.edit.as_ref().is_none_or(|e| e.owner != Some(b)) {
            let s = format!("{}{DASH}{} Hz", self.range_lo(st, b), self.range_hi(st, b));
            pen.text(&s, x0 + RANGE_X, HEADER_Y, VAL, if dim { hex(Pal::D_TEXT) } else { hex(Pal::INK2) }, Align2::LEFT_CENTER);
        }

        // S / M / B: 6 px squares, orange when on, a thin frame when off
        for (j, name) in ["S", "M", "B"].iter().enumerate() {
            let r = toggle_rect(b, j);
            let cy = r[1] + r[3] / 2.0;
            if self.on(st, bid(b, SOLO + j)) {
                pen.rect(r[0] + 2.0, cy - 3.0, 6.0, 6.0, hex(Pal::ACC));
            } else {
                pen.outline(r[0] + 2.0, cy - 3.0, 6.0, 6.0, hex(Pal::INK3));
            }
            pen.text(name, r[0] + 11.0, cy, LABEL, hex(Pal::INK), Align2::LEFT_CENTER);
        }

        // gain match: a small push button in a rounded groove, orange text while learning
        let r = match_rect(b);
        pen.pill(r[0], r[1], r[2], r[3], hex(Pal::OFF), None);
        let c = if dim {
            hex(Pal::D_TEXT)
        } else if self.params.shared.is_learning(b) {
            hex(Pal::ACC)
        } else {
            hex(Pal::INK)
        };
        pen.text("MATCH", r[0] + r[2] / 2.0, r[1] + r[3] / 2.0, LABEL, c, Align2::CENTER_CENTER);

        // gain reduction: orange cells hanging from the top
        let m = self.gr_meters[b];
        let cx = x0 + GR_X;
        let lit = ((m.shown / GR_METER_RANGE).clamp(0.0, 1.0) * FADER_ROWS as f32).round() as usize;
        let l = pen.lat(cx - 3.5, FADER_CELLS_Y, PITCH, CELL);
        for j in 0..FADER_ROWS {
            let c = if j < lit {
                if dim { hex(Pal::D_LIT) } else { hex(Pal::ACC) }
            } else {
                hex(Pal::OFF)
            };
            pen.cell(&l, 0, j, c);
            pen.cell(&l, 1, j, c);
        }
        pen.text("GR", cx, FADER_LABEL_Y, LABEL, ink, Align2::CENTER_CENTER);
        let reducing = !dim && m.readout >= 0.05;
        let (t, c) = if reducing {
            (format!("-{:.1} dB", m.readout), hex(Pal::ACC))
        } else {
            ("0.0 dB".to_owned(), if dim { hex(Pal::D_TEXT) } else { hex(Pal::INK3) })
        };
        pen.text(&t, cx, FADER_VALUE_Y, VAL, c, Align2::CENTER_CENTER);
    }

    fn draw_fader(&self, pen: &Pen<'_>, st: &Ctx, i: usize) {
        let f = self.faders[i];
        let v = self.plain(st, f.id);
        let dim = f.band.is_some_and(|b| self.bypassed(st, b));
        let dragging = self.drag.as_ref().is_some_and(|d| d.hit == Hit::Fader(i));
        let (lo_v, hi_v) = self.bounds(f.id);

        let n = f.cells();
        let r = (self.norm(f.id, v) * (n - 1) as f64).round() as usize;
        let bipolar = lo_v < 0.0 && hi_v > 0.0;
        let b0 = if bipolar { (self.norm(f.id, 0.0) * (n - 1) as f64).round() as usize } else { 0 };
        let (lo, hi) = (b0.min(r), b0.max(r));
        let cap = if dim { hex(Pal::D_KNOB) } else { hex(Pal::INK) };
        let lit = if dim { hex(Pal::D_LIT) } else { hex(Pal::LIT) };
        let text_c = if dim { hex(Pal::D_TEXT) } else { hex(Pal::INK) };
        let needle = if dragging { 1.0 } else { 0.5 };
        let editing = self.edit.as_ref().is_some_and(|e| e.target == EditTarget::Fader(i));

        if f.vertical {
            let l = pen.lat(f.x - 3.5, FADER_CELLS_Y, PITCH, CELL);
            for j in 0..n {
                let c = if j == r {
                    cap
                } else if j >= lo && j <= hi {
                    lit
                } else {
                    hex(Pal::OFF)
                };
                pen.cell(&l, 0, n - 1 - j, c);
                pen.cell(&l, 1, n - 1 - j, c);
            }
            pen.line_h(f.x - 9.0, f.x + 9.0, l.y(n - 1 - r) + l.cell() * 0.5, cap, needle);
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
                    hex(Pal::OFF)
                };
                pen.cell(&l, j, 0, c);
            }
            pen.line_v(l.x(r) + l.cell() * 0.5, cy - 5.0, cy + 5.0, cap, needle);
            pen.text(f.label, f.x + 4.0, cy, LABEL, text_c, Align2::LEFT_CENTER);
            if !editing {
                pen.text(&format_value(v, f.format), f.x + 224.0, cy, VAL, text_c, Align2::RIGHT_CENTER);
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
            color: hex(Pal::ACC),
            extra_letter_spacing: 0.8 * s,
            ..Default::default()
        };
        let g_head = painter.layout_job(egui::text::LayoutJob::single_section(head.to_uppercase(), head_fmt));
        let mut body_job = egui::text::LayoutJob::single_section(
            body.to_owned(),
            egui::TextFormat {
                font_id: FontId::new(10.5 * s, FontFamily::Proportional),
                color: hex(Pal::PLOT),
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
        painter.rect_filled(r, (3.0 * s) as u8, Color32::from_rgba_unmultiplied(0x11, 0x12, 0x10, 245));
        let y = p.y + pad.y;
        painter.galley(pos2(p.x + pad.x, y), g_head.clone(), hex(Pal::ACC));
        painter.galley(pos2(p.x + pad.x, y + g_head.size().y + 4.0 * s), g_body, hex(Pal::PLOT));
    }

    /// Value entry: Enter confirms, Esc or clicking elsewhere cancels.
    fn draw_edit(&mut self, ui: &mut egui::Ui, st: &Ctx, origin: Pos2, s: f32) {
        let Some(mut e) = self.edit.take() else { return };
        let r = Rect::from_min_size(pos2(origin.x + e.rect[0] * s, origin.y + e.rect[1] * s), vec2(e.rect[2] * s, e.rect[3] * s));
        ui.painter().rect_filled(r, 0.0, hex(Pal::PLOT));
        ui.painter().rect_stroke(r, 0.0, Stroke::new(1.0_f32, hex(Pal::RULE)), StrokeKind::Inside);
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
                .text_color(hex(Pal::INK)),
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
        if enter {
            self.apply_edit(st, &e);
        } else if !(esc || resp.lost_focus()) {
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
        let full = ui.max_rect();
        let s = (full.width() / W).min(full.height() / H);
        let origin = full.min;
        // remember the size, but only sizes the user can pick (100 – 180 %)
        let w = full.width().round() as u32;
        let (lo, hi) = SCALE_RANGE;
        if w as f32 >= W * lo - 1.0 && w as f32 <= W * hi + 1.0 && self.params.ui_width.load() != w {
            self.params.ui_width.store(w);
        }

        // own the whole surface so egui widgets underneath never see the clicks
        let _ = ui.allocate_rect(full, egui::Sense::click_and_drag());
        ui.painter().rect_filled(full, 0.0, hex(Pal::CELL));

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
        self.draw(&pen, st);
        // the resize corner: a staircase of cells
        let over = self.hot == Some(Hit::Resize) || self.drag.as_ref().is_some_and(|d| d.hit == Hit::Resize);
        for r in 0..3 {
            for c in (2 - r)..3 {
                pen.rect(W - 13.0 + 4.0 * c as f32, H - 13.0 + 4.0 * r as f32, 3.0, 3.0, if over { hex(Pal::INK2) } else { hex(Pal::INK3) });
            }
        }
        ui.visuals_mut().selection.bg_fill = Color32::from_rgba_unmultiplied(0xFF, 0x4D, 0x0F, 64);
        ui.visuals_mut().selection.stroke = Stroke::new(1.0_f32, hex(Pal::INK));
        ui.visuals_mut().text_cursor.stroke = Stroke::new(1.0_f32, hex(Pal::INK));
        self.draw_edit(ui, st, origin, s);
        self.draw_tooltip(&ctx, origin, s, full);
    }
}

/// Light-mode visuals with linear glyph coverage (JUCE renders coverage linearly).
fn visuals() -> egui::Visuals {
    let mut v = egui::Visuals::light();
    v.text_options.alpha_from_coverage = egui::epaint::AlphaFromCoverage::Linear;
    v
}

/// The size MBC4 wants: the saved width (within 100 – 180 %), or 150 % the first time.
fn wanted_size(params: &Mbc4Params) -> (u32, u32) {
    let stored = params.ui_width.load();
    let (lo, hi) = SCALE_RANGE;
    let w = if stored == 0 { (W * DEFAULT_SCALE) as u32 } else { stored.clamp((W * lo) as u32, (W * hi) as u32) };
    (w, (w as f32 * H / W).round() as u32)
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
        .aspect_ratio(Some((31, 20)))
        .min_size(((W * lo) as u32, (H * lo) as u32))
        .max_size(((W * hi) as u32, (H * hi) as u32))
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
        let (_, st, mut ui, ctx) = setup();
        let mut out = None;
        for _ in 0..3 {
            out = Some(frame(&ctx, &mut ui, &st, vec![]));
        }
        let t = texts(&out.unwrap());
        for s in ["INPUT", "SPLIT", "OUTPUT", "BAND A", "BAND D", "THS", "RAT", "MATCH", "GR", "LOOK", "IN+OUT", "0.0 dB", "2:1", "60 dB"] {
            assert!(t.iter().any(|x| x == s), "{s} missing: {t:?}");
        }
        assert!(t.iter().any(|x| x == "120\u{2013}1.00k Hz"), "band B range missing: {t:?}");
    }

    #[test]
    fn tooltip_paints_heading_and_japanese_body() {
        let (_, st, mut ui, ctx) = setup();
        // hover band A's THS fader body
        let p = pos2((strip_x(0) + THS_X) * 1.5, 500.0 * 1.5);
        for _ in 0..3 {
            frame(&ctx, &mut ui, &st, vec![Event::PointerMoved(p)]);
        }
        std::thread::sleep(std::time::Duration::from_millis(600));
        let out = frame(&ctx, &mut ui, &st, vec![Event::PointerMoved(p)]);
        let t = texts(&out);
        assert!(t.iter().any(|s| s == "THRESHOLD"), "tooltip heading missing: {t:?}");
        let body = t.iter().find(|s| s.contains("圧縮します")).expect("tooltip body missing");
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
