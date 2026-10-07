#include "PluginEditor.h"
#include "UI/Grid.h"

namespace
{
    constexpr double minScale = 0.75, maxScale = 2.0, defaultScale = 1.0;
}

//==============================================================================
MainView::MainView (MBC4Processor& p)
    : proc (p),
      graph (p),
      inGain  (p.apvts, "in_gain",  "in"),
      mix     (p.apvts, "mix",      "mix"),
      outGain (p.apvts, "out_gain", "out"),
      look    (p.apvts, "lookahead", "look")
{
    addAndMakeVisible (graph);
    graph.onBandSelected = [this] (int b) { selectBand (b); };

    for (int b = 0; b < MBC4Processor::numBands; ++b)
    {
        strips[(size_t) b] = std::make_unique<BandStrip> (p, b);
        strips[(size_t) b]->onSelect = [this] (int i) { selectBand (i); };
        addAndMakeVisible (*strips[(size_t) b]);
    }

    for (auto* k : { &inGain, &mix, &outGain, &look })
        addAndMakeVisible (*k);

    // analyzer source (segmented)
    const std::pair<juce::TextButton*, GraphView::AnalyzerMode> modes[] = {
        { &anBoth, GraphView::AnalyzerMode::both }, { &anPost, GraphView::AnalyzerMode::post },
        { &anPre,  GraphView::AnalyzerMode::pre },  { &anOff,  GraphView::AnalyzerMode::off } };

    for (auto& [btn, m] : modes)
    {
        btn->setRadioGroupId (1);
        btn->setClickingTogglesState (true);
        btn->onClick = [this, m = m] { graph.setAnalyzerMode (m); proc.apvts.state.setProperty ("analyzer", (int) m, nullptr); };
        addAndMakeVisible (*btn);
    }

    const int savedMode = juce::jlimit (0, 3, (int) proc.apvts.state.getProperty ("analyzer", 0));
    graph.setAnalyzerMode ((GraphView::AnalyzerMode) savedMode);
    modes[savedMode].first->setToggleState (true, juce::dontSendNotification);

    // segmented group
    anBoth.setConnectedEdges (juce::Button::ConnectedOnRight);
    anPost.setConnectedEdges (juce::Button::ConnectedOnLeft | juce::Button::ConnectedOnRight);
    anPre.setConnectedEdges (juce::Button::ConnectedOnLeft | juce::Button::ConnectedOnRight);
    anOff.setConnectedEdges (juce::Button::ConnectedOnLeft);
    anBoth.setTooltip ("analyzer: input + output");
    anPost.setTooltip ("analyzer: output");
    anPre.setTooltip ("analyzer: input");
    anOff.setTooltip ("analyzer off");

    selectBand ((int) proc.apvts.state.getProperty ("selBand", 0));

    setSize (baseW, baseH);
    startTimerHz (30);
}

MainView::~MainView()
{
    stopTimer();
}

void MainView::selectBand (int b)
{
    selected = juce::jlimit (0, MBC4Processor::numBands - 1, b);
    proc.apvts.state.setProperty ("selBand", selected, nullptr);

    graph.setSelectedBand (selected);
    for (int i = 0; i < MBC4Processor::numBands; ++i)
        strips[(size_t) i]->setSelected (i == selected);
}

void MainView::timerCallback()
{
    graph.updateFrame();
    for (auto& s : strips)
        s->updateMeter();


    auto ballistics = [] (float& shown, float& hold, int& holdTime, float v)
    {
        shown = v > shown ? v : juce::jmax (v, shown - 1.2f);
        if (v >= hold) { hold = v; holdTime = 45; }
        else if (--holdTime < 0) hold = juce::jmax (v, hold - 0.6f);
    };
    ballistics (inShown,  inHold,  inHoldTime,  proc.inPeakDb.load());
    ballistics (outShown, outHold, outHoldTime, proc.outPeakDb.load());
    repaint (meterArea);
}

void MainView::resized()
{
    using namespace Grid;
    auto r = getLocalBounds().reduced (margin);

    // top bar: segmented analyzer switch, one grid cell per segment
    topBar = r.removeFromTop (topBarH);
    {
        auto t = topBar;
        for (auto* b : { &anBoth, &anPost, &anPre, &anOff })
        {
            b->setBounds (t.removeFromLeft (segW));
            t.removeFromLeft (-1);   // segments share their border
        }
    }
    r.removeFromTop (gutter);

    // columns: main area | side panel
    sidePanel = r.removeFromRight (sideW);
    r.removeFromRight (gutter);

    graph.setBounds (r.removeFromTop (graphH));
    r.removeFromTop (gutter);

    const int stripW = (r.getWidth() - gutter * 3) / 4;
    for (int b = 0; b < MBC4Processor::numBands; ++b)
    {
        strips[(size_t) b]->setBounds (r.removeFromLeft (stripW));
        r.removeFromLeft (gutter);
    }

    // side knobs share the rows of the band knobs:
    //   in | out       (gain staging)
    //   mix            (centred)
    //   look           (centred)
    const int stripTop = strips[0]->getY();
    auto sideCell = [&] (int row, int col)   // col -1 = centred
    {
        const int x = col < 0 ? sidePanel.getCentreX() - cellW / 2
                              : sidePanel.getX() + (sideW - 2 * cellW) / 2 + col * cellW;
        return juce::Rectangle<int> (x, stripTop + knobTop + row * rowH, cellW, rowH);
    };
    for (auto* k : { &inGain, &mix, &outGain, &look })
        k->setMaxDiameter (knobD);

    inGain.setBounds  (sideCell (0, 0));
    outGain.setBounds (sideCell (0, 1));
    mix.setBounds     (sideCell (1, -1));
    look.setBounds    (sideCell (2, -1));

    // side meters span the graph's plot and share its dB scale
    const auto plotArea = graph.getPlotArea().translated ((float) graph.getX(), (float) graph.getY());
    meterArea = juce::Rectangle<int> (sidePanel.getX(), graph.getY(), sideW, graphH);
    meterPlot = plotArea.withX ((float) sidePanel.getX()).withWidth ((float) sideW);

    renderShadows();
}

void MainView::renderShadows()
{
    // drawn once per layout: blurring every frame would cost far more than the plugin itself
    const float scale = 2.0f;
    shadows = juce::Image (juce::Image::ARGB, juce::roundToInt ((float) getWidth() * scale),
                           juce::roundToInt ((float) getHeight() * scale), true);
    juce::Graphics g (shadows);
    g.addTransform (juce::AffineTransform::scale (scale));

    std::vector<juce::Rectangle<int>> blocks { graph.getBounds(), sidePanel };
    for (auto& s : strips)
        blocks.push_back (s->getBounds());

    for (auto& b : blocks)
    {
        juce::Path shape;
        shape.addRoundedRectangle (b.toFloat(), Palette::radius);
        Palette::blockShadow().drawForPath (g, shape);
        Palette::blockContact().drawForPath (g, shape);
    }
}

void MainView::paintMeters (juce::Graphics& g)
{
    using namespace Grid;
    const auto m = meterPlot;
    const float cIn = m.getX() + 4.5f * u, cOut = m.getRight() - 4.5f * u;   // bar centres
    const auto inBar  = juce::Rectangle<float> (cIn  - meterW * 0.5f, m.getY(), (float) meterW, m.getHeight());
    const auto outBar = juce::Rectangle<float> (cOut - meterW * 0.5f, m.getY(), (float) meterW, m.getHeight());

    constexpr float lo = GraphView::minDb, hi = GraphView::maxDb;
    auto yOf = [&] (float db) { return juce::jmap (juce::jlimit (lo, hi, db), lo, hi, m.getBottom(), m.getY()); };

    // the graph's dB grid lines, continued between the bars
    g.setFont (Fonts::ui (11.0f));
    for (float db = 0.0f; db >= lo; db -= 12.0f)
    {
        const float y = yOf (db);
        g.setColour (Palette::lineHi);
        g.fillRect (juce::Rectangle<float> (inBar.getRight() + 4.0f, y - Palette::hair * 0.5f, 4.0f, Palette::hair));
        g.fillRect (juce::Rectangle<float> (outBar.getX() - 8.0f, y - Palette::hair * 0.5f, 4.0f, Palette::hair));
        g.setColour (db == 0.0f ? Palette::text : Palette::soft);
        g.drawText (juce::String ((int) db), juce::Rectangle<float> (cIn, y - 8.0f, cOut - cIn, 16.0f), juce::Justification::centred, false);
    }

    auto bar = [&] (juce::Rectangle<float> b, float level, float hold, const char* label)
    {
        g.setColour (Palette::line);
        g.fillRect (b);

        // soft ink up to 0 dB, full ink above it
        const float y0 = yOf (0.0f);
        const float yl = yOf (level);
        g.setColour (Palette::text.withAlpha (0.55f));
        g.fillRect (juce::Rectangle<float>::leftTopRightBottom (b.getX(), juce::jmax (yl, y0), b.getRight(), b.getBottom()));
        if (yl < y0)
        {
            g.setColour (Palette::text);
            g.fillRect (juce::Rectangle<float>::leftTopRightBottom (b.getX(), yl, b.getRight(), y0));
        }

        if (hold > lo)
        {
            g.setColour (Palette::text);
            g.fillRect (juce::Rectangle<float> (b.getX() - 2.0f, yOf (hold) - 0.375f, b.getWidth() + 4.0f, 0.75f));
        }

        g.setFont (Fonts::ui (12.0f));
        g.setColour (Palette::text);
        // label in the graph's frequency-label row, held value in its tag row
        g.drawText (label, juce::Rectangle<float> (b.getCentreX() - 24.0f, m.getBottom() + 4.0f, 48.0f, 16.0f), juce::Justification::centred, false);
        g.drawText (hold <= -99.0f ? juce::String ("-inf") : juce::String (hold, 1),
                    juce::Rectangle<float> (b.getCentreX() - 24.0f, m.getY() - plotTop * 0.5f - 8.0f, 48.0f, 16.0f), juce::Justification::centred, false);
    };
    bar (inBar,  inShown,  inHold,  "in");
    bar (outBar, outShown, outHold, "out");
}

void MainView::paint (juce::Graphics& g)
{
    g.fillAll (Palette::bg);
    g.drawImage (shadows, getLocalBounds().toFloat());

    g.setColour (Palette::panel);
    g.fillRoundedRectangle (sidePanel.toFloat(), Palette::radius);

    paintMeters (g);
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
    if (! restoring)
        proc.apvts.state.setProperty ("uiScale", scale, nullptr);
}
