#include "GraphView.h"
#include "MBCLookAndFeel.h"
#include "Grid.h"

namespace
{
    constexpr float tiltDbPerOct = 3.0f;

    juce::String shortFreq (float f)
    {
        return f >= 1000.0f ? juce::String (f / 1000.0f, f >= 10000.0f ? 1 : 2) + "k" : juce::String (juce::roundToInt (f));
    }
}

GraphView::GraphView (MBC4Processor& p) : proc (p)
{
    auto& s = proc.apvts;
    for (int i = 0; i < 3; ++i)
        xParam[i] = s.getParameter (MBC4Processor::xoverParamId (i));

    for (int b = 0; b < MBC4Processor::numBands; ++b)
    {
        threshP[b] = s.getParameter (MBC4Processor::bandParamId (b, "thresh"));
        ratioP[b]  = s.getParameter (MBC4Processor::bandParamId (b, "ratio"));
        soloP[b]   = s.getParameter (MBC4Processor::bandParamId (b, "solo"));
        muteP[b]   = s.getParameter (MBC4Processor::bandParamId (b, "mute"));
        bypassP[b] = s.getParameter (MBC4Processor::bandParamId (b, "bypass"));
    }

    fftBuf.resize ((size_t) fftSize * 2, 0.0f);
    scratch.resize ((size_t) AnalyzerFifo::capacity, 0.0f);
    lvlDisp.fill (-100.0f);
    lvlHold.fill (-100.0f);

    freqEditor.setFont (Fonts::ui (12.0f));
    freqEditor.setJustification (juce::Justification::centred);
    freqEditor.setInputRestrictions (10, "0123456789.kKhHzZ ");
    freqEditor.onReturnKey = [this] { commitCrossoverEdit(); };
    freqEditor.onFocusLost = [this] { commitCrossoverEdit(); };
    freqEditor.onEscapeKey = [this] { editingIndex = -1; freqEditor.setVisible (false); };
    addChildComponent (freqEditor);

    setOpaque (false);   // rounded corners show the window behind
}

//==============================================================================
float GraphView::freqToX (float f) const noexcept
{
    return plot.getX() + plot.getWidth() * std::log (f / minHz) / std::log (maxHz / minHz);
}

float GraphView::xToFreq (float x) const noexcept
{
    return minHz * std::pow (maxHz / minHz, (x - plot.getX()) / plot.getWidth());
}

float GraphView::dbToY (float db) const noexcept
{
    return juce::jmap (db, maxDb, minDb, plot.getY(), plot.getBottom());
}

float GraphView::crossoverHz (int i) const
{
    return xParam[i]->convertFrom0to1 (xParam[i]->getValue());
}

float GraphView::bandLeftX (int b) const   { return b == 0 ? plot.getX() : freqToX (crossoverHz (b - 1)); }
float GraphView::bandRightX (int b) const  { return b == 3 ? plot.getRight() : freqToX (crossoverHz (b)); }

int GraphView::bandAt (float x) const
{
    for (int b = 0; b < 3; ++b)
        if (x < freqToX (crossoverHz (b)))
            return b;
    return 3;
}

juce::Rectangle<float> GraphView::tagBounds (int i) const
{
    return juce::Rectangle<float> (48.0f, 20.0f).withCentre ({ freqToX (crossoverHz (i)), plot.getY() - Grid::plotTop * 0.5f });
}

int GraphView::crossoverAt (juce::Point<float> p) const
{
    for (int i = 0; i < 3; ++i)
        if (tagBounds (i).contains (p))
            return i;

    if (! plot.contains (p))
        return -1;

    int best = -1;
    float bestDist = 6.0f;
    for (int i = 0; i < 3; ++i)
    {
        const float d = std::abs (freqToX (crossoverHz (i)) - p.x);
        if (d < bestDist) { bestDist = d; best = i; }
    }
    return best;
}

//==============================================================================
void GraphView::resized()
{
    auto r = getLocalBounds().toFloat();
    plot = r.withTrimmedRight ((float) Grid::plotRight).withTrimmedTop ((float) Grid::plotTop).withTrimmedBottom ((float) Grid::plotBottom);

    // one analyser point every 2 px
    const int numPoints = juce::jmax (2, (int) (plot.getWidth() / 2.0f));
    pointHz.resize ((size_t) numPoints);
    pointX.resize ((size_t) numPoints);
    for (int i = 0; i < numPoints; ++i)
    {
        pointX[(size_t) i]  = plot.getX() + plot.getWidth() * (float) i / (float) (numPoints - 1);
        pointHz[(size_t) i] = xToFreq (pointX[(size_t) i]);
    }

    for (auto* s : { &pre, &post })
        s->display.assign ((size_t) numPoints, minDb - 20.0f);

}

//==============================================================================
void GraphView::pullAndAnalyse (AnalyzerFifo& fifo, Spectrum& s)
{
    int got;
    while ((got = fifo.pull (scratch.data(), (int) scratch.size())) > 0)
    {
        for (int i = 0; i < got; ++i)
        {
            s.ring[(size_t) s.writePos] = scratch[(size_t) i];
            s.writePos = (s.writePos + 1) & (fftSize - 1);
        }
        s.newSamples += got;
    }

    const double sr = proc.getSampleRate();
    const auto numPoints = s.display.size();

    if (s.newSamples < fftSize / 4 || sr <= 0.0 || numPoints < 2)
    {
        // no fresh audio: let the curve fall
        for (auto& v : s.display)
            v = juce::jmax (minDb - 20.0f, v - 1.5f);
        return;
    }
    s.newSamples = 0;

    // unwrap the most recent fftSize samples
    for (int i = 0; i < fftSize; ++i)
        fftBuf[(size_t) i] = s.ring[(size_t) ((s.writePos + i) & (fftSize - 1))];
    std::fill (fftBuf.begin() + fftSize, fftBuf.end(), 0.0f);

    window.multiplyWithWindowingTable (fftBuf.data(), (size_t) fftSize);
    fft.performFrequencyOnlyForwardTransform (fftBuf.data(), true);

    const float binHz = (float) sr / (float) fftSize;
    const float norm  = 4.0f / (float) fftSize;   // hann: sine amplitude -> 1.0
    const int   maxBin = fftSize / 2 - 1;

    // 1/6-octave smoothing: average the power of all bins within +-1/12 octave
    static const float halfWidth = std::pow (2.0f, 1.0f / 12.0f);
    frame.resize (numPoints);

    for (size_t i = 0; i < numPoints; ++i)
    {
        const float f   = pointHz[i];
        const float pLo = f / halfWidth / binHz, pHi = f * halfWidth / binHz;

        float power;
        if (pHi - pLo < 2.0f)
        {
            // narrower than a bin (low end): interpolate
            const float pos = juce::jlimit (0.0f, (float) maxBin, f / binHz);
            const int   b0  = (int) pos;
            const int   b1  = juce::jmin (maxBin, b0 + 1);
            const float t   = pos - (float) b0;
            const float m   = fftBuf[(size_t) b0] + t * (fftBuf[(size_t) b1] - fftBuf[(size_t) b0]);
            power = m * m;
        }
        else
        {
            const int b0 = juce::jlimit (0, maxBin, (int) std::ceil (pLo));
            const int b1 = juce::jlimit (b0, maxBin, (int) pHi);
            power = 0.0f;
            for (int b = b0; b <= b1; ++b)
                power += fftBuf[(size_t) b] * fftBuf[(size_t) b];
            power /= (float) (b1 - b0 + 1);
        }

        frame[i] = juce::Decibels::gainToDecibels (std::sqrt (power) * norm, -140.0f)
                 + tiltDbPerOct * std::log2 (f / 1000.0f);
    }

    // light spatial smoothing across neighbouring points, then ballistics
    for (size_t i = 0; i < numPoints; ++i)
    {
        float acc = 0.0f, wsum = 0.0f;
        for (int k = -2; k <= 2; ++k)
        {
            const auto j = (size_t) juce::jlimit (0, (int) numPoints - 1, (int) i + k);
            const float wgt = (float) (3 - std::abs (k));
            acc += frame[j] * wgt; wsum += wgt;
        }
        const float db = acc / wsum;

        auto& v = s.display[i];
        v = db > v ? v + (db - v) * 0.5f : v + (db - v) * 0.14f;
    }
}

void GraphView::buildPath (Spectrum& s)
{
    s.path.clear();
    s.fill.clear();
    if (s.display.empty())
        return;

    const float bottom = plot.getBottom() + 2.0f;
    auto yAt = [&] (size_t i) { return juce::jlimit (plot.getY(), bottom, dbToY (s.display[i])); };

    // curve through the points: each point is a control point, the midpoints are on the curve
    s.path.startNewSubPath (pointX.front(), yAt (0));
    for (size_t i = 1; i + 1 < s.display.size(); ++i)
        s.path.quadraticTo (pointX[i], yAt (i), (pointX[i] + pointX[i + 1]) * 0.5f, (yAt (i) + yAt (i + 1)) * 0.5f);
    s.path.lineTo (pointX.back(), yAt (s.display.size() - 1));

    s.fill = s.path;
    s.fill.lineTo (pointX.back(), bottom);
    s.fill.lineTo (pointX.front(), bottom);
    s.fill.closeSubPath();
}

void GraphView::updateFrame()
{
    if (mode == AnalyzerMode::both || mode == AnalyzerMode::pre)   pullAndAnalyse (proc.preFifo, pre);
    else                                                            { float d[512]; while (proc.preFifo.pull (d, 512) > 0) {} }

    if (mode == AnalyzerMode::both || mode == AnalyzerMode::post)  pullAndAnalyse (proc.postFifo, post);
    else                                                            { float d[512]; while (proc.postFifo.pull (d, 512) > 0) {} }

    buildPath (pre);
    buildPath (post);

    for (size_t b = 0; b < (size_t) MBC4Processor::numBands; ++b)
    {
        const float gr = proc.engine.getGainReductionDb ((int) b);
        grDisp[b] = gr < grDisp[b] ? gr : grDisp[b] + (gr - grDisp[b]) * 0.25f;

        const float lv = proc.engine.getBandLevelDb ((int) b);
        lvlDisp[b] = lv > lvlDisp[b] ? lv : juce::jmax (lv, lvlDisp[b] - 0.8f);

        // slow, readable peak hold for the level marker
        if (lv >= lvlHold[b]) { lvlHold[b] = lv; lvlHoldTime[b] = 45; }
        else if (--lvlHoldTime[b] < 0) lvlHold[b] = juce::jmax (lv, lvlHold[b] - 0.25f);
    }

    repaint();
}

//==============================================================================
void GraphView::paint (juce::Graphics& g)
{
    // rounded block: fill, and keep everything inside the corners
    {
        juce::Path shape;
        shape.addRoundedRectangle (getLocalBounds().toFloat(), Palette::radius);
        g.reduceClipRegion (shape);
    }
    g.fillAll (Palette::plot);

    const auto ink  = Palette::text;
    const auto soft = Palette::soft;
    auto hline = [&] (float y, float x0, float x1) { g.fillRect (juce::Rectangle<float> (x0, y - Palette::hair * 0.5f, x1 - x0, Palette::hair)); };
    auto vline = [&] (float x, float y0, float y1) { g.fillRect (juce::Rectangle<float> (x - Palette::hair * 0.5f, y0, Palette::hair, y1 - y0)); };

    // ---- grid -------------------------------------------------------------
    g.setFont (Fonts::ui (11.0f));
    for (float f : { 30.0f, 40.0f, 50.0f, 60.0f, 70.0f, 80.0f, 90.0f, 100.0f, 200.0f, 300.0f, 400.0f, 500.0f, 600.0f, 700.0f, 800.0f, 900.0f,
                     1000.0f, 2000.0f, 3000.0f, 4000.0f, 5000.0f, 6000.0f, 7000.0f, 8000.0f, 9000.0f, 10000.0f })
    {
        const bool major = (f == 100.0f || f == 1000.0f || f == 10000.0f);
        const float x = freqToX (f);
        g.setColour (ink.withAlpha (major ? 0.13f : 0.05f));
        vline (x, plot.getY(), plot.getBottom());

        if (major || f == 50.0f || f == 500.0f || f == 5000.0f)
        {
            const auto txt = f >= 1000.0f ? juce::String ((int) (f / 1000.0f)) + "k" : juce::String ((int) f);
            g.setColour (soft);
            g.drawText (txt, juce::Rectangle<float> (x - 20.0f, plot.getBottom() + 4.0f, 40.0f, 16.0f), juce::Justification::centred);
        }
    }

    for (float db = 0.0f; db >= minDb; db -= 12.0f)
    {
        const float y = dbToY (db);
        g.setColour (ink.withAlpha (db == 0.0f ? 0.15f : 0.05f));
        hline (y, plot.getX(), plot.getRight());
        g.setColour (soft);
        g.drawText (juce::String ((int) db), juce::Rectangle<float> (plot.getRight() + 8.0f, y - 8.0f, 30.0f, 16.0f),
                    juce::Justification::centredLeft);
    }

    // ---- band regions -----------------------------------------------------
    bool anySolo = false;
    for (auto* s : soloP) anySolo = anySolo || s->getValue() > 0.5f;

    for (int b = 0; b < MBC4Processor::numBands; ++b)
    {
        const float x0 = bandLeftX (b), x1 = bandRightX (b);
        const auto region = juce::Rectangle<float> (x0, plot.getY(), x1 - x0, plot.getHeight());

    }

    // ---- spectrum ---------------------------------------------------------
    const bool showPre  = mode == AnalyzerMode::both || mode == AnalyzerMode::pre;
    const bool showPost = mode == AnalyzerMode::both || mode == AnalyzerMode::post;
    {
        juce::Graphics::ScopedSaveState ss (g);
        g.reduceClipRegion (plot.toNearestInt());

        if (showPost)
        {
            g.setGradientFill (juce::ColourGradient (ink.withAlpha (0.09f), 0.0f, plot.getY(),
                                                     ink.withAlpha (0.01f), 0.0f, plot.getBottom(), false));
            g.fillPath (post.fill);
            g.setColour (ink.withAlpha (0.85f));
            g.strokePath (post.path, juce::PathStrokeType (0.8f));
        }

        if (showPre)
        {
            g.setColour (mode == AnalyzerMode::pre ? ink.withAlpha (0.85f) : soft.withMultipliedAlpha (0.7f));
            g.strokePath (pre.path, juce::PathStrokeType (0.6f));
        }
    }

    // ---- per band overlays --------------------------------------------------
    const float pxPerDb = plot.getHeight() / (maxDb - minDb);

    for (int b = 0; b < MBC4Processor::numBands; ++b)
    {
        const float x0 = bandLeftX (b), x1 = bandRightX (b);
        const float w = x1 - x0;
        const float cx = (x0 + x1) * 0.5f;
        const bool bypassed = bypassP[b]->getValue() > 0.5f;
        const bool audible = muteP[b]->getValue() < 0.5f && (! anySolo || soloP[b]->getValue() > 0.5f);

        // muted / not soloed: wash the band out
        if (! audible)
        {
            g.setColour (Palette::plot.withAlpha (0.65f));
            g.fillRect (juce::Rectangle<float> (x0, plot.getY(), w, plot.getHeight()));
        }

        // band letter, top left of its section
        g.setColour (soft);
        g.setFont (Fonts::ui (12.0f));
        g.drawText (juce::String::charToString ((juce::juce_wchar) ('A' + b)),
                    juce::Rectangle<float> (x0 + 8.0f, plot.getY() + 8.0f, 16.0f, 16.0f), juce::Justification::centredLeft, false);

        // gain reduction, hanging from the 0 dB line
        const float gr = bypassed ? 0.0f : grDisp[(size_t) b];
        if (gr < -0.05f)
        {
            const float barW = juce::jlimit (4.0f, 30.0f, w * 0.2f);
            auto bar = juce::Rectangle<float> (cx - barW * 0.5f, dbToY (0.0f), barW, -gr * pxPerDb);
            g.setColour (ink.withAlpha (0.10f));
            g.fillRect (bar);
            g.setColour (ink);
            g.fillRect (bar.removeFromBottom (0.8f));
        }

        // detected band level: live line + held hairline
        const float lw = juce::jmax (6.0f, w * 0.36f);
        if (lvlDisp[(size_t) b] > minDb)
        {
            g.setColour (soft);
            g.fillRect (juce::Rectangle<float> (cx - lw * 0.5f, dbToY (lvlDisp[(size_t) b]) - 0.5f, lw, 1.0f));
        }
        if (lvlHold[(size_t) b] > minDb)
        {
            g.setColour (ink.withAlpha (0.6f));
            hline (dbToY (lvlHold[(size_t) b]), cx - lw * 0.5f, cx + lw * 0.5f);
        }

        // threshold
        const float thr = threshP[b]->convertFrom0to1 (threshP[b]->getValue());
        const float ty = dbToY (thr);
        g.setColour (bypassed ? soft.withMultipliedAlpha (0.6f) : ink.withAlpha (0.75f));
        g.fillRect (juce::Rectangle<float> (x0 + 1.0f, ty - 0.375f, w - 2.0f, 0.75f));

        // its value only while it is being looked at / moved
        const bool touching = (hoverBand == b && drag == Drag::none) || (drag == Drag::threshold && dragIndex == b);
        if (touching && w > 40.0f)
        {
            g.setColour (ink);
            g.setFont (Fonts::ui (12.0f));
            g.drawText (juce::String (thr, 1), juce::Rectangle<float> (x0 + 8.0f, ty + 4.0f, w - 16.0f, 16.0f),
                        juce::Justification::centredLeft, false);
        }
    }

    // ---- crossovers -------------------------------------------------------
    for (int i = 0; i < 3; ++i)
    {
        const float x = freqToX (crossoverHz (i));
        g.setColour (Palette::lineHi);
        vline (x, plot.getY() - 3.0f, plot.getBottom());

        const auto tag = tagBounds (i);
        g.setColour (ink);
        g.setFont (Fonts::ui (12.0f));
        g.drawText (shortFreq (crossoverHz (i)), tag, juce::Justification::centred, false);
    }

}

//==============================================================================
void GraphView::setParam (juce::RangedAudioParameter* p, float value)
{
    p->setValueNotifyingHost (p->convertTo0to1 (value));
}

void GraphView::editCrossover (int i)
{
    editingIndex = i;
    freqEditor.setBounds (tagBounds (i).expanded (8.0f, 1.0f).toNearestInt());
    freqEditor.setText (shortFreq (crossoverHz (i)), false);
    freqEditor.setVisible (true);
    freqEditor.grabKeyboardFocus();
    freqEditor.selectAll();
}

void GraphView::commitCrossoverEdit()
{
    if (editingIndex < 0)
        return;

    const int i = editingIndex;
    editingIndex = -1;
    freqEditor.setVisible (false);

    auto* p = xParam[i];
    const float typed = p->getValueForText (freqEditor.getText());   // normalised
    if (freqEditor.getText().trim().isEmpty())
        return;

    const float lo = i > 0 ? crossoverHz (i - 1) * 1.19f : minHz;
    const float hi = i < 2 ? crossoverHz (i + 1) / 1.19f : maxHz;
    p->beginChangeGesture();
    setParam (p, juce::jlimit (lo, hi, p->convertFrom0to1 (typed)));
    p->endChangeGesture();
    repaint();
}

void GraphView::mouseMove (const juce::MouseEvent& e)
{
    const int x = crossoverAt (e.position);
    const int b = plot.contains (e.position) ? bandAt (e.position.x) : -1;

    if (x != hoverX || b != hoverBand)
    {
        hoverX = x;
        hoverBand = b;
        repaint();
    }

    setMouseCursor (x >= 0 ? juce::MouseCursor::LeftRightResizeCursor
                           : b >= 0 ? juce::MouseCursor::UpDownResizeCursor : juce::MouseCursor::NormalCursor);
}

void GraphView::mouseExit (const juce::MouseEvent&)
{
    hoverX = hoverBand = -1;
    repaint();
}

void GraphView::mouseDown (const juce::MouseEvent& e)
{
    drag = Drag::none;
    dragParam = nullptr;
    lastDragY = e.position.y;

    if (const int x = crossoverAt (e.position); x >= 0)
    {
        if (e.mods.isAltDown())   // Alt+click a split: back to its default
        {
            auto* p = xParam[x];
            p->beginChangeGesture();
            p->setValueNotifyingHost (p->getDefaultValue());
            p->endChangeGesture();
            repaint();
            return;
        }

        drag = Drag::crossover;
        dragIndex = x;
        dragParam = xParam[x];
        dragParam->beginChangeGesture();
        return;
    }

    if (! plot.contains (e.position))
        return;

    const int b = bandAt (e.position.x);
    selected = b;
    if (onBandSelected)
        onBandSelected (b);

    drag = Drag::threshold;
    dragIndex = b;
    dragParam = threshP[b];
    dragStartValue = dragParam->convertFrom0to1 (dragParam->getValue());
    dragParam->beginChangeGesture();
    repaint();
}

void GraphView::mouseDrag (const juce::MouseEvent& e)
{
    if (dragParam == nullptr)
        return;

    if (drag == Drag::crossover)
    {
        const float lo = dragIndex > 0 ? crossoverHz (dragIndex - 1) * 1.19f : minHz;
        const float hi = dragIndex < 2 ? crossoverHz (dragIndex + 1) / 1.19f : maxHz;
        setParam (dragParam, juce::jlimit (lo, hi, xToFreq (e.position.x)));
        repaint();
        return;
    }

    // threshold: incremental so Ctrl (or Shift) can switch to fine mode mid-drag, same 1/6 as the bars
    const float pxPerDb = plot.getHeight() / (maxDb - minDb);
    const bool fineMode = e.mods.isCtrlDown() || e.mods.isCommandDown() || e.mods.isShiftDown();
    const float fine = fineMode ? 1.0f / 6.0f : 1.0f;
    const float delta = -(e.position.y - lastDragY) / pxPerDb * fine;
    lastDragY = e.position.y;

    dragStartValue = dragParam->getNormalisableRange().snapToLegalValue (dragStartValue + delta);
    setParam (dragParam, dragStartValue);
    repaint();
}

void GraphView::mouseUp (const juce::MouseEvent&)
{
    if (dragParam != nullptr)
        dragParam->endChangeGesture();

    dragParam = nullptr;
    drag = Drag::none;
    repaint();
}

void GraphView::mouseDoubleClick (const juce::MouseEvent& e)
{
    for (int i = 0; i < 3; ++i)
    {
        if (tagBounds (i).contains (e.position))
        {
            editCrossover (i);
            return;
        }
    }

    juce::RangedAudioParameter* p = nullptr;

    if (const int x = crossoverAt (e.position); x >= 0)
        p = xParam[x];
    else if (plot.contains (e.position))
        p = threshP[bandAt (e.position.x)];

    if (p != nullptr)
    {
        p->beginChangeGesture();
        p->setValueNotifyingHost (p->getDefaultValue());
        p->endChangeGesture();
        repaint();
    }
}
