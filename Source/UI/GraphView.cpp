#include "GraphView.h"
#include "MBCLookAndFeel.h"
#include "Grid.h"

namespace
{
    constexpr float tiltDbPerOct = 3.0f;
    constexpr float silentDb = Grid::minDb - 20.0f;

    juce::String hzText (float f)
    {
        return f >= 1000.0f ? juce::String (f / 1000.0f, f >= 10000.0f ? 1 : 2) + "kHz" : juce::String (juce::roundToInt (f)) + " Hz";
    }

    int rowsFor (float db)
    {
        return juce::jlimit (0, Grid::plotRows, (int) std::floor ((db - Grid::minDb) / Grid::dbPerRow + 1.0e-4f));
    }

    float roundSignificant (float f, int digits)
    {
        const float e = std::pow (10.0f, std::floor (std::log10 (f)) - (float) (digits - 1));
        return std::round (f / e) * e;
    }

    bool fineMods (const juce::ModifierKeys& m) { return m.isShiftDown() || m.isCtrlDown() || m.isCommandDown(); }
}

GraphView::GraphView (MBC4Processor& p) : proc (p)
{
    auto& s = proc.apvts;
    for (int i = 0; i < 3; ++i)
        xParam[i] = s.getParameter (MBC4Processor::xoverParamId (i));

    for (int b = 0; b < MBC4Processor::numBands; ++b)
    {
        threshP[b] = s.getParameter (MBC4Processor::bandParamId (b, "thresh"));
        bypassP[b] = s.getParameter (MBC4Processor::bandParamId (b, "bypass"));
    }

    fftBuf.resize ((size_t) fftSize * 2, 0.0f);
    scratch.resize ((size_t) AnalyzerFifo::capacity, 0.0f);
    pre.display.fill (silentDb);
    post.display.fill (silentDb);

    for (int c = 0; c < numCols; ++c)
        colHz[(size_t) c] = minHz * std::pow (maxHz / minHz, ((float) c + 0.5f) / (float) numCols);

    modePill.setTooltip (juce::String::fromUTF8 ("Analyzer\nスペクトラムの表示。IN+OUT / OUT / IN / OFF。"));
    modePill.onChange = [this] (int i)
    {
        mode = (AnalyzerMode) i;
        if (onModeChanged)
            onModeChanged (mode);
        repaint();
    };
    addAndMakeVisible (modePill);
}

void GraphView::setAnalyzerMode (AnalyzerMode m)
{
    mode = m;
    modePill.setSelected ((int) m, false);
    repaint();
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
    return juce::jmap (db, Grid::maxDb, Grid::minDb, plot.getY(), plot.getBottom());
}

float GraphView::yToDb (float y) const noexcept
{
    return juce::jmap (y, plot.getY(), plot.getBottom(), Grid::maxDb, Grid::minDb);
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

int GraphView::crossoverAt (juce::Point<float> p) const
{
    if (p.y < plot.getY() || p.y > plot.getBottom())
        return -1;

    int best = -1;
    float bestDist = 4.5f;
    for (int i = 0; i < 3; ++i)
    {
        const float d = std::abs (freqToX (crossoverHz (i)) - p.x);
        if (d < bestDist) { bestDist = d; best = i; }
    }
    return best;
}

bool GraphView::nearThreshold (juce::Point<float> p, int band) const
{
    const float thr = threshP[band]->convertFrom0to1 (threshP[band]->getValue());
    return std::abs (p.y - dbToY (thr)) <= 4.0f;
}

//==============================================================================
void GraphView::resized()
{
    // window coordinates -> local
    plot = juce::Rectangle<float> ((float) (Grid::plotL - getX()), (float) (Grid::plotT - getY()),
                                   (float) (Grid::plotR - Grid::plotL), (float) (Grid::plotB - Grid::plotT));

    const int w = modePill.getIdealWidth();
    modePill.setBounds (Grid::colR - 10 - getX() - w, 7, w, 16);
}

//==============================================================================
void GraphView::drainFifo (AnalyzerFifo& fifo)
{
    while (fifo.pull (scratch.data(), (int) scratch.size()) > 0) {}
}

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

    if (s.newSamples < fftSize / 4 || sr <= 0.0)
    {
        // no fresh audio: let the spectrum fall
        for (auto& v : s.display)
            v = juce::jmax (silentDb, v - 1.5f);
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

    for (int i = 0; i < numCols; ++i)
    {
        const float f   = colHz[(size_t) i];
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

        frame[(size_t) i] = juce::Decibels::gainToDecibels (std::sqrt (power) * norm, -140.0f)
                          + tiltDbPerOct * std::log2 (f / 1000.0f);
    }

    // light smoothing across neighbouring columns, then ballistics
    for (int i = 0; i < numCols; ++i)
    {
        float acc = 0.0f, wsum = 0.0f;
        for (int k = -2; k <= 2; ++k)
        {
            const auto j = (size_t) juce::jlimit (0, numCols - 1, i + k);
            const float wgt = (float) (3 - std::abs (k));
            acc += frame[j] * wgt; wsum += wgt;
        }
        const float db = acc / wsum;

        auto& v = s.display[(size_t) i];
        v = db > v ? v + (db - v) * 0.5f : v + (db - v) * 0.14f;
    }
}

void GraphView::updateFrame()
{
    // the input is also needed in "out" mode: it shows how much each band removes
    if (mode != AnalyzerMode::off) pullAndAnalyse (proc.preFifo, pre);
    else                           drainFifo (proc.preFifo);

    if (mode == AnalyzerMode::both || mode == AnalyzerMode::post) pullAndAnalyse (proc.postFifo, post);
    else                                                          drainFifo (proc.postFifo);

    for (size_t b = 0; b < (size_t) MBC4Processor::numBands; ++b)
    {
        const float gr = proc.engine.getGainReductionDb ((int) b);
        grDisp[b] = gr < grDisp[b] ? gr : grDisp[b] + (gr - grDisp[b]) * 0.25f;
    }

    repaint();
}

//==============================================================================
void GraphView::paintCells (juce::Graphics& g)
{
    const Draw::Px px (g, *this);
    const bool showOut = mode == AnalyzerMode::both || mode == AnalyzerMode::post;
    const bool showIn  = mode != AnalyzerMode::off;

    // one rectangle list per colour; runs of cells are filled as single rectangles
    // and the 1 px gaps between cells are drawn over them afterwards
    constexpr int numColours = (int) Palette::ramp.size() + 3;   // ramp, accent, ghost, off
    std::array<juce::RectangleList<float>, numColours> lists;
    const int accentIdx = (int) Palette::ramp.size(), ghostIdx = accentIdx + 1, offIdx = accentIdx + 2;

    auto rampIndex = [] (int j) { return juce::jlimit (0, (int) Palette::ramp.size() - 1,
                                                       (int) ((float) j / (float) numRows * (float) Palette::ramp.size() * 1.15f)); };

    for (int c = 0; c < numCols; ++c)
    {
        const int b = bandAt (freqToX (colHz[(size_t) c]));
        const float inDb = pre.display[(size_t) c];
        const float gr = bypassP[b]->getValue() > 0.5f ? 0.0f : grDisp[(size_t) b];

        const int nIn  = showIn  ? rowsFor (inDb) : 0;
        const int nOut = showOut ? rowsFor (post.display[(size_t) c]) : 0;
        const int nCut = rowsFor (inDb + gr);

        auto colourOf = [&] (int j)
        {
            if (mode == AnalyzerMode::pre)
                return j < nIn ? rampIndex (j) : offIdx;
            if (j < nOut)                return rampIndex (j);
            if (j < nIn && j >= nCut)    return accentIdx;   // removed by the compressor
            if (j < nIn && showIn && mode == AnalyzerMode::both) return ghostIdx;
            return offIdx;
        };

        const float x = plot.getX() + Grid::pitch * (float) c;
        int runStart = 0, runColour = colourOf (0);
        for (int j = 1; j <= numRows; ++j)
        {
            const int col = j < numRows ? colourOf (j) : -1;
            if (col == runColour)
                continue;

            // rows [runStart, j) counted from the bottom
            const float top = plot.getY() + Grid::pitch * (float) (numRows - j);
            lists[(size_t) runColour].addWithoutMerging (px.rect (x, top, Grid::cell, Grid::pitch * (float) (j - runStart)));
            runStart = j;
            runColour = col;
        }
    }

    g.setColour (Palette::off);    g.fillRectList (lists[(size_t) offIdx]);
    g.setColour (Palette::ghost);  g.fillRectList (lists[(size_t) ghostIdx]);
    for (size_t i = 0; i < Palette::ramp.size(); ++i)
    {
        g.setColour (Palette::ramp[i]);
        g.fillRectList (lists[i]);
    }
    g.setColour (Palette::accent); g.fillRectList (lists[(size_t) accentIdx]);

    // gaps between cells, in the background colour
    juce::RectangleList<float> gaps;
    for (int c = 0; c < numCols; ++c)
    {
        const float x = plot.getX() + Grid::pitch * (float) c + Grid::cell;
        const float x0 = px.snapX (x);
        gaps.addWithoutMerging ({ x0, plot.getY(), px.snapX (x + 1.0f) - x0, plot.getHeight() });
    }
    for (int r = 0; r < numRows; ++r)
    {
        const float y = plot.getY() + Grid::pitch * (float) r + Grid::cell;
        const float y0 = px.snapY (y);
        gaps.addWithoutMerging ({ plot.getX(), y0, plot.getWidth(), px.snapY (y + 1.0f) - y0 });
    }
    g.setColour (Palette::cell);
    g.fillRectList (gaps);
}

void GraphView::paint (juce::Graphics& g)
{
    const Draw::Px px (g, *this);
    using J = juce::Justification;
    using R = Fonts::Role;

    Draw::sectionHeader (g, "02", "SPLIT", 10.0f, 15.0f);

    paintCells (g);

    // scale: numbers only
    for (float db = 0.0f; db >= Grid::minDb; db -= 12.0f)
        Draw::text (g, juce::String ((int) db), plot.getRight() + 7.0f, dbToY (db), R::tick, Palette::ink2);

    const std::pair<float, const char*> freqs[] = { { 50.0f, "50" }, { 100.0f, "100" }, { 200.0f, "200" }, { 500.0f, "500" },
                                                    { 1000.0f, "1k" }, { 2000.0f, "2k" }, { 5000.0f, "5k" }, { 10000.0f, "10k" } };
    const float labelY = (float) (Grid::readoutTop - getY());
    for (auto& [f, t] : freqs)
        Draw::text (g, t, freqToX (f), labelY, R::tick, Palette::ink2, J::centred);

    // crossovers
    for (int i = 0; i < 3; ++i)
    {
        const bool on = (drag == Drag::crossover && dragIndex == i) || (drag == Drag::none && hot == Hot::crossover && hotIndex == i);
        Draw::lineV (g, px, freqToX (crossoverHz (i)), plot.getY(), plot.getBottom(), Palette::ink, on ? 1.2f : 0.5f);
    }

    // thresholds + band letters
    for (int b = 0; b < MBC4Processor::numBands; ++b)
    {
        const float x0 = bandLeftX (b), x1 = bandRightX (b);
        const bool bypassed = bypassP[b]->getValue() > 0.5f;
        const bool on = (drag == Drag::threshold && dragIndex == b) || (drag == Drag::none && hot == Hot::threshold && hotIndex == b);
        const float thr = threshP[b]->convertFrom0to1 (threshP[b]->getValue());

        Draw::lineH (g, px, x0, x1, dbToY (thr), bypassed ? Palette::dKnob : Palette::ink, on ? 1.2f : 0.5f);
        Draw::text (g, juce::String::charToString ((juce::juce_wchar) ('A' + b)), x0 + 3.0f, plot.getY() - 10.0f,
                    R::label, bypassed ? Palette::dText : Palette::ink2);
    }

    // value bubble for whatever is under the mouse / being moved
    const auto kind  = drag == Drag::crossover ? Hot::crossover : drag == Drag::threshold ? Hot::threshold : hot;
    const int  index = drag != Drag::none ? dragIndex : hotIndex;

    if (kind == Hot::threshold && index >= 0)
    {
        const float thr = threshP[index]->convertFrom0to1 (threshP[index]->getValue());
        Draw::bubble (g, juce::String::charToString ((juce::juce_wchar) ('A' + index)) + "  " + formatValue (thr, Format::dB),
                      mouse.x, dbToY (thr) - 12.0f, plot.getX(), plot.getRight());
    }
    else if (kind == Hot::crossover && index >= 0)
    {
        Draw::bubble (g, hzText (crossoverHz (index)), freqToX (crossoverHz (index)), plot.getY() + 10.0f, plot.getX(), plot.getRight());
    }
    else if (kind == Hot::plot)
    {
        Draw::bubble (g, hzText (xToFreq (mouse.x)) + "  " + numberText (yToDb (mouse.y)) + " dB",
                      mouse.x, mouse.y - 14.0f, plot.getX(), plot.getRight());
    }
}

//==============================================================================
void GraphView::setParam (juce::RangedAudioParameter* p, float value)
{
    p->setValueNotifyingHost (p->convertTo0to1 (value));
}

void GraphView::mouseMove (const juce::MouseEvent& e)
{
    mouse = e.position;
    auto newHot = Hot::none;
    int newIndex = -1;

    if (const int x = crossoverAt (e.position); x >= 0)
    {
        newHot = Hot::crossover;
        newIndex = x;
    }
    else if (plot.contains (e.position))
    {
        const int b = bandAt (e.position.x);
        newHot = nearThreshold (e.position, b) ? Hot::threshold : Hot::plot;
        newIndex = newHot == Hot::threshold ? b : -1;
    }

    hot = newHot;
    hotIndex = newIndex;
    setMouseCursor (hot == Hot::crossover ? juce::MouseCursor::LeftRightResizeCursor
                  : hot != Hot::none     ? juce::MouseCursor::UpDownResizeCursor : juce::MouseCursor::NormalCursor);
    repaint();
}

void GraphView::mouseExit (const juce::MouseEvent&)
{
    hot = Hot::none;
    hotIndex = -1;
    repaint();
}

void GraphView::mouseDown (const juce::MouseEvent& e)
{
    drag = Drag::none;
    dragParam = nullptr;
    mouse = e.position;
    lastDragY = e.position.y;

    if (const int x = crossoverAt (e.position); x >= 0)
    {
        drag = Drag::crossover;
        dragIndex = x;
        dragParam = xParam[x];
        dragParam->beginChangeGesture();
        return;
    }

    if (! plot.contains (e.position))
        return;

    // anywhere inside a band moves its threshold
    const int b = bandAt (e.position.x);
    drag = Drag::threshold;
    dragIndex = b;
    dragParam = threshP[b];
    dragRaw = dragParam->convertFrom0to1 (dragParam->getValue());
    dragParam->beginChangeGesture();
    repaint();
}

void GraphView::mouseDrag (const juce::MouseEvent& e)
{
    mouse = e.position;
    if (dragParam == nullptr)
        return;

    const bool fine = fineMods (e.mods);

    if (drag == Drag::crossover)
    {
        const float lo = dragIndex > 0 ? crossoverHz (dragIndex - 1) * 1.19f : minHz;
        const float hi = dragIndex < 2 ? crossoverHz (dragIndex + 1) / 1.19f : maxHz;
        const float f  = roundSignificant (juce::jlimit (minHz, maxHz, xToFreq (e.position.x)), fine ? 3 : 2);
        setParam (dragParam, juce::jlimit (lo, hi, f));
        repaint();
        return;
    }

    // threshold: incremental, so the fine modifier can be pressed mid-drag
    const float pxPerDb = plot.getHeight() / (Grid::maxDb - Grid::minDb);
    dragRaw = juce::jlimit (-60.0f, 0.0f, dragRaw - (e.position.y - lastDragY) / pxPerDb * (fine ? 0.2f : 1.0f));
    lastDragY = e.position.y;

    const float step = fine ? 0.1f : 1.0f;
    setParam (dragParam, std::round (dragRaw / step) * step);
    repaint();
}

void GraphView::mouseUp (const juce::MouseEvent& e)
{
    if (dragParam != nullptr)
        dragParam->endChangeGesture();

    dragParam = nullptr;
    drag = Drag::none;
    mouseMove (e);
}

void GraphView::mouseDoubleClick (const juce::MouseEvent& e)
{
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
