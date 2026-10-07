#include "BandStrip.h"
#include "MBCLookAndFeel.h"
#include "Grid.h"

//==============================================================================
void FineSlider::mouseDown (const juce::MouseEvent& e)
{
    altClick = e.mods.isAltDown();
    if (altClick)
    {
        if (onAltClick)
            onAltClick();
        return;
    }

    Slider::mouseDown (e);   // gesture start
    lastPos = e.position;
    dragProportion = valueToProportionOfLength (getValue());
}

void FineSlider::mouseDrag (const juce::MouseEvent& e)
{
    if (altClick || ! isEnabled() || e.mods.isPopupMenu())
        return;

    const float delta = (e.position.x - lastPos.x) + (lastPos.y - e.position.y);
    lastPos = e.position;

    const bool fine = e.mods.isCtrlDown() || e.mods.isCommandDown() || e.mods.isShiftDown();
    const double pixelsForFullRange = fine ? 230.0 * 6.0 : 230.0;

    // the drag position itself stays continuous; only plain drags snap to the detents
    dragProportion = juce::jlimit (0.0, 1.0, dragProportion + delta / pixelsForFullRange);
    const double target = fine ? dragProportion
                               : std::round (dragProportion * detents) / (double) detents;
    setValue (proportionOfLengthToValue (target), juce::sendNotificationSync);
}

void FineSlider::mouseUp (const juce::MouseEvent& e)
{
    if (! altClick)
        Slider::mouseUp (e);   // gesture end
    altClick = false;
}

void FineSlider::mouseDoubleClick (const juce::MouseEvent&)
{
    if (onDoubleClick)
        onDoubleClick();
}

//==============================================================================
Knob::Knob (juce::AudioProcessorValueTreeState& state, const juce::String& paramId, const juce::String& n)
    : name (n)
{
    slider.setSliderStyle (juce::Slider::RotaryHorizontalVerticalDrag);
    slider.setRotaryParameters (juce::MathConstants<float>::pi * 1.25f, juce::MathConstants<float>::pi * 2.75f, true);
    slider.setTextBoxStyle (juce::Slider::NoTextBox, false, 0, 0);
    slider.setSliderSnapsToMousePosition (false);
    addAndMakeVisible (slider);

    attachment = std::make_unique<juce::AudioProcessorValueTreeState::SliderAttachment> (state, paramId, slider);

    auto* param = state.getParameter (paramId);
    slider.onAltClick = [this, param]
    {
        if (param != nullptr)
            slider.setValue (param->convertFrom0to1 (param->getDefaultValue()), juce::sendNotificationSync);
    };
    slider.onDoubleClick = [this]
    {
        value.setVisible (true);
        value.showEditor();
    };

    value.setJustificationType (juce::Justification::centred);
    value.setFont (Fonts::ui (12.0f));
    value.setColour (juce::Label::textColourId, Palette::text);
    value.onTextChange = [this] { slider.setValue (slider.getValueFromText (value.getText()), juce::sendNotificationSync); refreshText(); };
    value.onEditorHide = [this] { updateHover(); };
    addChildComponent (value);

    slider.onValueChange = [this] { refreshText(); };
    slider.addMouseListener (this, false);
    refreshText();
}

Knob::~Knob()
{
    slider.removeMouseListener (this);
}

void Knob::refreshText()
{
    value.setText (slider.getTextFromValue (slider.getValue()).toLowerCase(), juce::dontSendNotification);
}

void Knob::updateHover()
{
    const bool show = slider.isMouseOver (true) || slider.isMouseButtonDown() || value.isBeingEdited();
    if (show != value.isVisible())
    {
        value.setVisible (show);
        repaint();
    }
}

void Knob::paint (juce::Graphics& g)
{
    if (! value.isVisible())
    {
        g.setColour (Palette::text);
        g.setFont (Fonts::ui (12.0f));
        g.drawText (name, value.getBounds(), juce::Justification::centred, false);
    }
}

void Knob::resized()
{
    // disc and its label as one block on the grid, centred in the cell
    const auto r = getLocalBounds();
    const int d = juce::jmin (maxDiameter, r.getWidth(), r.getHeight() - Grid::labelH - Grid::labelGap);
    const int top = r.getY() + (r.getHeight() - (d + Grid::labelGap + Grid::labelH)) / 2;

    slider.setBounds (r.getCentreX() - d / 2, top, d, d);
    value.setBounds (r.getX(), top + d + Grid::labelGap, r.getWidth(), Grid::labelH);
}

//==============================================================================
namespace
{
    constexpr float meterRange = 24.0f;

    juce::String freqLabel (float f)
    {
        return f >= 1000.0f ? juce::String (f / 1000.0f, f >= 10000.0f ? 1 : 2) + "k" : juce::String (juce::roundToInt (f));
    }
}

BandStrip::BandStrip (MBC4Processor& p, int b)
    : proc (p), band (b),
      thresh  (p.apvts, MBC4Processor::bandParamId (b, "thresh"),  "ths"),
      ratio   (p.apvts, MBC4Processor::bandParamId (b, "ratio"),   "rat"),
      knee    (p.apvts, MBC4Processor::bandParamId (b, "knee"),    "kne"),
      attack  (p.apvts, MBC4Processor::bandParamId (b, "attack"),  "atk"),
      release (p.apvts, MBC4Processor::bandParamId (b, "release"), "rls"),
      range   (p.apvts, MBC4Processor::bandParamId (b, "range"),   "rng"),
      mix     (p.apvts, MBC4Processor::bandParamId (b, "mix"),     "mix"),
      gain    (p.apvts, MBC4Processor::bandParamId (b, "gain"),    "gain")
{
    for (auto* k : knobs())
    {
        k->setMaxDiameter (Grid::knobD);
        addAndMakeVisible (*k);
    }

    solo.setTooltip ("solo");
    mute.setTooltip ("mute");
    bypass.setTooltip ("bypass dynamics");
    for (auto* btn : { &solo, &mute, &bypass })
    {
        btn->setClickingTogglesState (true);
        addAndMakeVisible (*btn);
    }

    using BA = juce::AudioProcessorValueTreeState::ButtonAttachment;
    soloAtt   = std::make_unique<BA> (p.apvts, MBC4Processor::bandParamId (b, "solo"), solo);
    muteAtt   = std::make_unique<BA> (p.apvts, MBC4Processor::bandParamId (b, "mute"), mute);
    bypassAtt = std::make_unique<BA> (p.apvts, MBC4Processor::bandParamId (b, "bypass"), bypass);

    // gain match: learn the average gain reduction once and write the compensation into gain
    learn.setTooltip ("gain match: measures the average gain reduction over 3 s and sets gain once. click again to cancel.");
    learn.onClick = [this] { toggleLearn(); };
    addAndMakeVisible (learn);

    // band edges: the low edge is crossover b-1, the high edge crossover b
    for (auto* l : { &loFreq, &hiFreq })
    {
        l->setFont (Fonts::ui (12.0f));
        l->setColour (juce::Label::textColourId, Palette::text);
        l->setJustificationType (juce::Justification::centredLeft);
        addAndMakeVisible (*l);
    }

    if (band > 0)
    {
        loFreq.setEditable (false, true, false);
        loFreq.onTextChange = [this] { setCrossoverFromText (band - 1, loFreq.getText()); };
    }
    if (band < 3)
    {
        hiFreq.setEditable (false, true, false);
        hiFreq.onTextChange = [this] { setCrossoverFromText (band, hiFreq.getText()); };
    }
}

//==============================================================================
void BandStrip::toggleLearn()
{
    if (proc.engine.isLearning (band))
        proc.engine.stopLearning (band);
    else
        proc.engine.startLearning (band);

    updateLearn();
}

void BandStrip::updateLearn()
{
    if (! proc.engine.isLearning (band))
    {
        learn.setToggleState (false, juce::dontSendNotification);
        learn.setButtonText ("match");
        return;
    }

    const double secs = proc.engine.learnedSeconds (band);
    if (secs >= learnSeconds)
    {
        // done: store the compensation once; it does not move afterwards
        proc.engine.stopLearning (band);
        const float avg = proc.engine.learnedAverageDb (band);
        if (auto* p = proc.apvts.getParameter (MBC4Processor::bandParamId (band, "gain")))
        {
            p->beginChangeGesture();
            p->setValueNotifyingHost (p->convertTo0to1 (juce::jlimit (0.0f, 24.0f, -avg)));
            p->endChangeGesture();
        }
        learn.setToggleState (false, juce::dontSendNotification);
        learn.setButtonText ("match");
        return;
    }

    learn.setToggleState (true, juce::dontSendNotification);
    learn.setButtonText (juce::String (juce::roundToInt (100.0 * secs / learnSeconds)) + "%");
}

void BandStrip::setCrossoverFromText (int index, const juce::String& text)
{
    auto* p = proc.apvts.getParameter (MBC4Processor::xoverParamId (index));
    auto hz = [this] (int i) { return proc.apvts.getRawParameterValue (MBC4Processor::xoverParamId (i))->load(); };

    if (text.trim().isNotEmpty())
    {
        const float lo = index > 0 ? hz (index - 1) * 1.19f : 20.0f;
        const float hi = index < 2 ? hz (index + 1) / 1.19f : 20000.0f;
        const float f  = juce::jlimit (lo, hi, p->convertFrom0to1 (p->getValueForText (text)));
        p->beginChangeGesture();
        p->setValueNotifyingHost (p->convertTo0to1 (f));
        p->endChangeGesture();
    }

    lastLo = lastHi = -1.0f;   // force the labels to refresh
}

void BandStrip::layoutFreqRow()
{
    // labels hug their text so the row reads "120 - 1.00k hz"
    auto width = [] (const juce::Label& l)
    {
        juce::GlyphArrangement ga;
        ga.addLineOfText (l.getFont(), l.getText(), 0.0f, 0.0f);
        return juce::roundToInt (ga.getBoundingBox (0, -1, true).getWidth()) + 6;
    };

    auto fr = freqRow;
    loFreq.setBounds (fr.removeFromLeft (width (loFreq)));
    fr.removeFromLeft (12);
    hiFreq.setBounds (fr.removeFromLeft (width (hiFreq)));
}

void BandStrip::setSelected (bool s)
{
    if (selected != s)
    {
        selected = s;
        repaint();
    }
}

void BandStrip::mouseDown (const juce::MouseEvent&)
{
    if (onSelect)
        onSelect (band);
}

void BandStrip::updateMeter()
{
    const float gr = juce::jmax (-meterRange, proc.engine.getGainReductionDb (band));

    const float prevGr = grShown, prevPeak = grPeak;
    grShown = gr < grShown ? gr : grShown + (gr - grShown) * 0.25f;

    // held value: stays 1.5 s, then falls slowly so it can be read
    if (gr <= grPeak) { grPeak = gr; peakHold = 45; }
    else if (--peakHold < 0) grPeak = juce::jmin (0.0f, grPeak + 0.15f);

    if (std::abs (prevGr - grShown) > 0.01f || std::abs (prevPeak - grPeak) > 0.01f)
        repaint (meterArea.expanded (48, 20));

    if (proc.engine.isLearning (band) || learn.getToggleState())
        updateLearn();

    // header follows the crossovers
    auto hz = [this] (int i) { return proc.apvts.getRawParameterValue (MBC4Processor::xoverParamId (i))->load(); };
    const float lo = band == 0 ? 20.0f : hz (band - 1);
    const float hi = band == 3 ? 20000.0f : hz (band);
    if (lo != lastLo || hi != lastHi)
    {
        lastLo = lo; lastHi = hi;
        if (! loFreq.isBeingEdited()) loFreq.setText (freqLabel (lo), juce::dontSendNotification);
        if (! hiFreq.isBeingEdited()) hiFreq.setText (freqLabel (hi), juce::dontSendNotification);
        layoutFreqRow();
        repaint (freqRow);
    }
}

//==============================================================================
void BandStrip::resized()
{
    using namespace Grid;
    const int left = pad, right = getWidth() - pad;

    // header row: letter | frequency range ........ s m b
    header = { left, pad, right - left, headerH };
    // s m b, small square buttons on half-unit spacing
    int bx = right;
    for (auto* btn : { &bypass, &mute, &solo })
    {
        bx -= smallButtonD;
        btn->setBounds (bx, pad + (headerH - smallButtonD) / 2, smallButtonD, smallButtonD);
        bx -= u / 2;
    }
    // text starts at the left edge of the first knob disc
    const int textLeft = left + (cellW - knobD) / 2;
    letterArea = { textLeft, pad, 2 * u, headerH };
    freqRow    = { textLeft + 2 * u, pad, bx - (textLeft + 2 * u), headerH };
    layoutFreqRow();

    // knob grid: 3 columns x 3 rows of fixed cells
    auto cell = [&] (int col, int row) { return juce::Rectangle<int> (left + col * cellW, knobTop + row * rowH, cellW, rowH); };
    const std::array<std::array<Knob*, 3>, 3> layout { { { &thresh, &ratio, &knee },
                                                         { &attack, &release, &range },
                                                         { &mix, &gain, nullptr } } };
    for (int row = 0; row < 3; ++row)
        for (int col = 0; col < 3; ++col)
            if (auto* k = layout[(size_t) row][(size_t) col])
                k->setBounds (cell (col, row));

    // match sits on the disc line of the last cell
    const auto last = cell (2, 2);
    learn.setBounds (juce::Rectangle<int> (6 * u, 20).withCentre ({ last.getCentreX(), last.getY() + knobInset + knobD / 2 }));

    // GR meter: right edge aligned with the buttons, spanning first disc top .. last label bottom
    const int meterTop = knobTop + knobInset;
    const int meterBottom = knobTop + 2 * rowH + knobInset + knobBlock;
    meterArea = { right - meterW, meterTop, meterW, meterBottom - meterTop };
}

void BandStrip::paint (juce::Graphics& g)
{
    const auto bounds = getLocalBounds().toFloat();

    g.setColour (Palette::panel);
    g.fillRoundedRectangle (bounds, Palette::radius);

    g.setColour (Palette::text);
    g.setFont (Fonts::ui (12.0f));
    g.drawText (juce::String::charToString ((juce::juce_wchar) ('A' + band)), letterArea, juce::Justification::centredLeft, false);
    g.drawText ("-", loFreq.getBounds().withX (loFreq.getRight()).withWidth (12), juce::Justification::centred, false);
    g.drawText ("hz", hiFreq.getBounds().withX (hiFreq.getRight() + 1).withWidth (24), juce::Justification::centredLeft, false);

    // gain reduction meter (0 .. -24 dB, hanging from the top)
    const auto m = meterArea.toFloat();
    auto yOf = [&] (float db) { return m.getY() + m.getHeight() * juce::jlimit (0.0f, 1.0f, -db / meterRange); };

    g.setColour (Palette::line);
    g.fillRect (m);
    g.setColour (Palette::text);
    g.fillRect (m.withBottom (juce::jmax (m.getY(), yOf (grShown))));

    g.setFont (Fonts::ui (11.0f));
    g.setColour (Palette::soft);
    g.drawText ("GR", juce::Rectangle<float> (m.getRight() - 24.0f, m.getY() - 20.0f, 24.0f, 16.0f), juce::Justification::centredRight, false);

    // held value next to its line
    if (grPeak < -0.05f)
    {
        const float py = yOf (grPeak);
        g.setColour (Palette::text);
        g.fillRect (juce::Rectangle<float> (m.getX() - 4.0f, py - 0.375f, m.getWidth() + 4.0f, 0.75f));
        g.setFont (Fonts::ui (12.0f));
        const float ty = juce::jlimit (m.getY(), m.getBottom() - 16.0f, py - 8.0f);
        g.drawText (juce::String (grPeak, 1), juce::Rectangle<float> (m.getX() - 48.0f, ty, 40.0f, 16.0f),
                    juce::Justification::centredRight, false);
    }
}
