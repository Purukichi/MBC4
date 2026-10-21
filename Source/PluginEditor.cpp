#include "PluginEditor.h"
#include "UI/Grid.h"

namespace
{
    constexpr double minScale = 1.0, maxScale = 1.8, defaultScale = 1.5;

    float meterY (float db)
    {
        return juce::jmap (db, Grid::maxDb, Grid::minDb, (float) Grid::plotT, (float) Grid::plotB);
    }

    int rowsFor (float db, int rows)
    {
        const float perRow = (Grid::maxDb - Grid::minDb) / (float) rows;
        return juce::jlimit (0, rows, (int) std::floor ((db - Grid::minDb) / perRow + 1.0e-4f));
    }
}

//==============================================================================
void MainView::Meter::update (float v)
{
    shown = v > shown ? v : juce::jmax (v, shown - 1.2f);
    // held value: stays 1.5 s, then falls slowly so it can be read
    if (v >= hold) { hold = v; holdTime = 45; }
    else if (--holdTime < 0) hold = juce::jmax (v, hold - 0.6f);
}

MainView::MainView (MBC4Processor& p)
    : proc (p),
      graph (p),
      inGain  (p.apvts, "in_gain",   "IN",   CellFader::Orientation::vertical, { 1.0f, 0.1f, {} }, Format::dB),
      look    (p.apvts, "lookahead", "LOOK", CellFader::Orientation::vertical, { 0.5f, 0.1f, {} }, Format::ms),
      mix     (p.apvts, "mix",       "MIX",  CellFader::Orientation::vertical, { 5.0f, 1.0f, {} }, Format::percent),
      outGain (p.apvts, "out_gain",  "OUT",  CellFader::Orientation::vertical, { 1.0f, 0.1f, {} }, Format::dB)
{
    addAndMakeVisible (graph);
    graph.onModeChanged = [this] (GraphView::AnalyzerMode m) { proc.apvts.state.setProperty ("analyzer", (int) m, nullptr); };
    graph.setAnalyzerMode ((GraphView::AnalyzerMode) juce::jlimit (0, 3, (int) proc.apvts.state.getProperty ("analyzer", 0)));

    for (int b = 0; b < MBC4Processor::numBands; ++b)
    {
        strips[(size_t) b] = std::make_unique<BandStrip> (p, b);
        addAndMakeVisible (*strips[(size_t) b]);
    }

    inGain.setTooltip  (tooltipText ("Input", "入力ゲイン。"));
    look.setTooltip    (tooltipText ("Lookahead", "先読み時間。レイテンシとしてホストに報告します。"));
    mix.setTooltip     (tooltipText ("Mix", "全体のドライ／ウェットの割合。"));
    outGain.setTooltip (tooltipText ("Output", "出力ゲイン。"));
    for (auto* f : { &inGain, &look, &mix, &outGain })
        addAndMakeVisible (*f);

    setSize (baseW, baseH);
    startTimerHz (30);
}

MainView::~MainView()
{
    stopTimer();
}

void MainView::timerCallback()
{
    graph.updateFrame();
    for (auto& s : strips)
        s->updateMeter();

    inMeter.update (proc.inPeakDb.load());
    outMeter.update (proc.outPeakDb.load());
    repaint (Grid::frameL, Grid::plotT - 2, Grid::colL - Grid::frameL, Grid::readoutTop + 20 - Grid::plotT);
    repaint (Grid::colR, Grid::plotT - 2, Grid::frameR - Grid::colR, Grid::readoutTop + 20 - Grid::plotT);
}

void MainView::resized()
{
    using namespace Grid;
    graph.setBounds (colL, frameT, colR - colL, rowB - frameT);

    for (int b = 0; b < MBC4Processor::numBands; ++b)
        strips[(size_t) b]->setBounds (colL + stripW * b, rowB, stripW, frameB - rowB);

    auto vfader = [] (CellFader& f, int cx) { f.setBounds (cx - faderW / 2, faderTop, faderW, faderH); };
    vfader (inGain, 80);
    vfader (look,    1124);
    vfader (mix,     1160);
    vfader (outGain, 1196);
}

//==============================================================================
void MainView::paintMeter (juce::Graphics& g, const Draw::Px& px, float cx, const Meter& m)
{
    // two columns of dots on the graph's rows (same lattice); above 0 dB in orange
    const Draw::Lattice lat (px, cx - 3.5f, (float) Grid::plotT, Grid::pitch, Grid::cell);
    const int rows = lat.fit ((float) (Grid::plotB - Grid::plotT));
    const int n = rowsFor (m.shown, rows), ref = rowsFor (0.0f, rows);
    juce::RectangleList<float> off, over;
    std::array<juce::RectangleList<float>, Palette::ramp.size()> ramp;

    for (int j = 0; j < rows; ++j)
    {
        const int ri = juce::jlimit (0, (int) Palette::ramp.size() - 1,
                                     (int) ((float) j / (float) rows * (float) Palette::ramp.size() * 1.15f));
        auto& list = j >= n ? off : j >= ref ? over : ramp[(size_t) ri];
        list.addWithoutMerging (lat.at (0, rows - 1 - j));
        list.addWithoutMerging (lat.at (1, rows - 1 - j));
    }

    g.setColour (Palette::off);
    g.fillRectList (off);
    for (size_t i = 0; i < ramp.size(); ++i)
    {
        g.setColour (Palette::ramp[i]);
        g.fillRectList (ramp[i]);
    }
    g.setColour (Palette::accent);
    g.fillRectList (over);

    // 0 dB reference: exactly as wide as the dots
    Draw::lineH (g, px, lat.x (0), lat.x (0) + lat.extent (2), meterY (0.0f), Palette::ink);

    const auto readout = m.hold <= Grid::minDb ? juce::String ("--.-")
                                               : (m.hold > 0.0f ? "+" : "") + juce::String (m.hold, 1);
    Draw::dotNumber (g, px, readout, cx, (float) Grid::readoutTop, m.hold > 0.0f ? Palette::accent : Palette::ink);
}

void MainView::paint (juce::Graphics& g)
{
    using namespace Grid;
    const Draw::Px px (g, *this);
    g.fillAll (Palette::cell);

    // blocks are separated by rules only
    Draw::ruleH (g, frameL, frameR, frameT);
    Draw::ruleH (g, frameL, frameR, frameB);
    Draw::ruleV (g, frameL, frameT, frameB);
    Draw::ruleV (g, frameR, frameT, frameB);
    Draw::ruleV (g, colL, frameT, frameB);
    Draw::ruleV (g, colR, frameT, frameB);
    Draw::ruleH (g, frameL, colL, logoB);
    Draw::ruleH (g, colL, colR, rowB);
    for (int i = 1; i < MBC4Processor::numBands; ++i)
        Draw::ruleV (g, (float) (colL + stripW * i), rowB, frameB);

    Draw::logo (g, px, 30.0f, 25.5f);
    Draw::sectionHeader (g, "01", "INPUT", 30.0f, 65.0f);
    Draw::sectionHeader (g, "07", "OUTPUT", (float) colR + 10.0f, 35.0f);

    paintMeter (g, px, 80.0f, inMeter);
    paintMeter (g, px, 1160.0f, outMeter);

    Draw::text (g, "V" + juce::String (JucePlugin_VersionString), (float) frameL, 771.0f, Fonts::Role::note, Palette::ink3);
}

//==============================================================================
MBC4Editor::MBC4Editor (MBC4Processor& p)
    : AudioProcessorEditor (p), proc (p), view (p)
{
    setLookAndFeel (&laf);
    addAndMakeVisible (view);

    // read the saved scale first: setting the limits below resizes the editor,
    // and resized() would otherwise overwrite the stored value
    const double scale = juce::jlimit (minScale, maxScale, (double) proc.apvts.state.getProperty ("uiScale", defaultScale));

    // the whole UI scales with the window (fixed aspect ratio)
    setResizable (true, true);
    setResizeLimits (juce::roundToInt (MainView::baseW * minScale), juce::roundToInt (MainView::baseH * minScale),
                     juce::roundToInt (MainView::baseW * maxScale), juce::roundToInt (MainView::baseH * maxScale));
    getConstrainer()->setFixedAspectRatio ((double) MainView::baseW / (double) MainView::baseH);

    setSize (juce::roundToInt (MainView::baseW * scale), juce::roundToInt (MainView::baseH * scale));
    restoring = false;
}

MBC4Editor::~MBC4Editor()
{
    setLookAndFeel (nullptr);
}

void MBC4Editor::resized()
{
    const float scale = (float) getWidth() / (float) MainView::baseW;
    view.setTransform (juce::AffineTransform::scale (scale));
    laf.setUiScale (scale);
    if (! restoring)
        proc.apvts.state.setProperty ("uiScale", scale, nullptr);
}
