#include "Controls.h"
#include "MBCLookAndFeel.h"
#include "Grid.h"

juce::String numberText (float v)
{
    const float r = std::round (v * 10.0f) / 10.0f;
    if (std::abs (r) < 0.05f)
        return "0";
    return std::abs (r - std::round (r)) < 0.01f ? juce::String ((int) std::round (r)) : juce::String (r, 1);
}

juce::String formatValue (float v, Format f)
{
    switch (f)
    {
        case Format::dB:      return numberText (v) + " dB";
        case Format::ms:      return (v < 10.0f ? juce::String (std::round (v * 10.0f) / 10.0f, 1) : juce::String (juce::roundToInt (v))) + " ms";
        case Format::percent: return juce::String (juce::roundToInt (v)) + " %";
        case Format::ratio:   return v >= 19.95f ? juce::String ("20:1") : numberText (v) + ":1";
    }
    return numberText (v);
}

//==============================================================================
namespace
{
    // vertical: 44 x 202 (label row, 40 cells, value row); horizontal: 228 x 14
    constexpr float vCx = 22.0f, vLabelY = 7.0f, vCellsTop = 22.0f, vValueY = 194.0f;
    constexpr float hLabelX = 4.0f, hCellsX = 40.0f, hValueR = 224.0f;
    constexpr float dragPixelsV = 160.0f, dragPixelsH = 128.0f, fineFactor = 0.2f;

    bool fineMods (const juce::ModifierKeys& m) { return m.isShiftDown() || m.isCtrlDown() || m.isCommandDown(); }
}

CellFader::CellFader (juce::AudioProcessorValueTreeState& state, const juce::String& paramId, const juce::String& l,
                      Orientation o, Steps s, Format f)
    : param (*state.getParameter (paramId)),
      label (l), orientation (o), steps (std::move (s)), format (f),
      attachment (param, [this] (float v) { value = v; repaint(); }, state.undoManager)
{
    attachment.sendInitialUpdate();

    editor.setFont (Fonts::of (Fonts::Role::value));
    editor.setJustification (vertical() ? juce::Justification::centred : juce::Justification::centredRight);
    editor.setIndents (2, 0);
    editor.setBorder ({});
    editor.onReturnKey = [this] { closeEditor (true); };
    editor.onEscapeKey = [this] { closeEditor (false); };
    editor.onFocusLost = [this] { closeEditor (false); };
    addChildComponent (editor);
}

int CellFader::numCells() const noexcept
{
    return vertical() ? Grid::faderRows : Grid::sliderCells;
}

juce::Rectangle<float> CellFader::valueArea() const
{
    return vertical() ? juce::Rectangle<float> (0.0f, vValueY - 7.0f, (float) getWidth(), 14.0f)
                      : juce::Rectangle<float> (hValueR - 50.0f, 0.0f, 50.0f, (float) getHeight());
}

void CellFader::resized()
{
    editor.setBounds (valueArea().toNearestInt());
}

bool CellFader::hitTest (int x, int y)
{
    juce::ignoreUnused (y);
    // vertical faders paint wider than they grab, so neighbours can sit close
    return ! vertical() || std::abs ((float) x - vCx) <= Grid::faderGrab * 0.5f;
}

void CellFader::paint (juce::Graphics& g)
{
    const Draw::Px px (g, *this);
    const bool dim = isDimmed != nullptr && isDimmed();
    const auto& range = param.getNormalisableRange();

    const int n = numCells();
    const int r = juce::roundToInt (range.convertTo0to1 (value) * (float) (n - 1));
    const bool bipolar = range.start < 0.0f && range.end > 0.0f;
    const int b0 = bipolar ? juce::roundToInt (range.convertTo0to1 (0.0f) * (float) (n - 1)) : 0;
    const int lo = juce::jmin (b0, r), hi = juce::jmax (b0, r);

    juce::RectangleList<float> offCells, litCells, cap;
    const auto capColour = dim ? Palette::dKnob : Palette::ink;
    const auto textColour = dim ? Palette::dText : Palette::ink;

    if (vertical())
    {
        for (int j = 0; j < n; ++j)
        {
            auto& list = j == r ? cap : (j >= lo && j <= hi ? litCells : offCells);
            const float y = vCellsTop + Grid::pitch * (float) (n - 1 - j);
            list.addWithoutMerging (px.cell (vCx - 3.5f, y));
            list.addWithoutMerging (px.cell (vCx + 0.5f, y));
        }
    }
    else
    {
        const float y = (float) getHeight() * 0.5f - 1.5f;
        for (int j = 0; j < n; ++j)
            (j == r ? cap : (j < r ? litCells : offCells)).addWithoutMerging (px.cell (hCellsX + Grid::pitch * (float) j, y));
    }

    g.setColour (Palette::off);              g.fillRectList (offCells);
    g.setColour (dim ? Palette::dLit : Palette::lit); g.fillRectList (litCells);
    g.setColour (capColour);                 g.fillRectList (cap);

    const float needle = dragging ? 1.0f : 0.5f;
    using J = juce::Justification;
    if (vertical())
    {
        Draw::lineH (g, px, vCx - 9.0f, vCx + 9.0f, vCellsTop + Grid::pitch * (float) (n - 1 - r) + 1.5f, capColour, needle);
        Draw::text (g, label, vCx, vLabelY, Fonts::Role::label, textColour, J::centred);
        if (! editor.isVisible())
            Draw::text (g, formatValue (value, format), vCx, vValueY, Fonts::Role::value, textColour, J::centred);
    }
    else
    {
        const float cy = (float) getHeight() * 0.5f;
        Draw::lineV (g, px, hCellsX + Grid::pitch * (float) r + 1.5f, cy - 5.0f, cy + 5.0f, capColour, needle);
        Draw::text (g, label, hLabelX, cy, Fonts::Role::label, textColour);
        if (! editor.isVisible())
            Draw::text (g, formatValue (value, format), hValueR, cy, Fonts::Role::value, textColour, J::right);
    }
}

//==============================================================================
float CellFader::snap (float v, bool fine) const
{
    const auto& range = param.getNormalisableRange();
    v = juce::jlimit (range.start, range.end, v);

    if (fine)
        return juce::jlimit (range.start, range.end, std::round (v / steps.fine) * steps.fine);

    if (! steps.list.empty())
    {
        const float n = range.convertTo0to1 (v);
        float best = steps.list.front();
        for (float s : steps.list)
            if (std::abs (range.convertTo0to1 (s) - n) < std::abs (range.convertTo0to1 (best) - n))
                best = s;
        return best;
    }

    return juce::jlimit (range.start, range.end, std::round (v / steps.step) * steps.step);
}

void CellFader::setFromUser (float v)
{
    if (v != value)
        attachment.setValueAsPartOfGesture (v);
}

void CellFader::mouseDown (const juce::MouseEvent& e)
{
    if (editor.isVisible())
        return;

    anchor = e.position;
    anchorNorm = param.getNormalisableRange().convertTo0to1 (value);
    dragFine = fineMods (e.mods);
    dragging = true;
    attachment.beginGesture();
    repaint();
}

void CellFader::mouseDrag (const juce::MouseEvent& e)
{
    if (! dragging)
        return;

    const bool fine = fineMods (e.mods);
    if (fine != dragFine)
    {
        anchor = e.position;
        anchorNorm = param.getNormalisableRange().convertTo0to1 (value);
        dragFine = fine;
    }

    const float d = vertical() ? (anchor.y - e.position.y) / dragPixelsV : (e.position.x - anchor.x) / dragPixelsH;
    const float n = juce::jlimit (0.0f, 1.0f, anchorNorm + d * (fine ? fineFactor : 1.0f));
    setFromUser (snap (param.getNormalisableRange().convertFrom0to1 (n), fine));
}

void CellFader::mouseUp (const juce::MouseEvent&)
{
    if (! dragging)
        return;

    dragging = false;
    attachment.endGesture();
    repaint();
}

void CellFader::mouseDoubleClick (const juce::MouseEvent& e)
{
    if (valueArea().contains (e.position))
    {
        if (dragging) { dragging = false; attachment.endGesture(); }
        openEditor();
        return;
    }

    // the second click already opened a gesture
    setFromUser (param.convertFrom0to1 (param.getDefaultValue()));
}

void CellFader::mouseWheelMove (const juce::MouseEvent& e, const juce::MouseWheelDetails& w)
{
    const float delta = std::abs (w.deltaY) > std::abs (w.deltaX) ? w.deltaY : -w.deltaX;
    if (delta == 0.0f || editor.isVisible())
        return;

    const int dir = (delta > 0.0f) != w.isReversed ? 1 : -1;
    const bool fine = fineMods (e.mods);

    float v;
    if (! steps.list.empty() && ! fine)
    {
        const float cur = snap (value, false);
        int idx = 0;
        for (int i = 0; i < (int) steps.list.size(); ++i)
            if (steps.list[(size_t) i] == cur)
                idx = i;
        v = steps.list[(size_t) juce::jlimit (0, (int) steps.list.size() - 1, idx + dir)];
    }
    else
    {
        v = snap (value + (float) dir * (fine ? steps.fine : steps.step), fine);
    }

    if (v != value)
        attachment.setValueAsCompleteGesture (v);
}

void CellFader::openEditor()
{
    editor.setText (numberText (value), false);
    editor.setVisible (true);
    editor.grabKeyboardFocus();
    editor.selectAll();
    repaint();
}

void CellFader::closeEditor (bool commit)
{
    if (! editor.isVisible())
        return;

    const auto t = editor.getText().retainCharacters ("0123456789.-");
    editor.setVisible (false);

    if (commit && t.isNotEmpty())
    {
        const auto& range = param.getNormalisableRange();
        attachment.setValueAsCompleteGesture (juce::jlimit (range.start, range.end, t.getFloatValue()));
    }
    repaint();
}

//==============================================================================
void SquareToggle::paintButton (juce::Graphics& g, bool, bool)
{
    const Draw::Px px (g, *this);
    const float cy = (float) getHeight() * 0.5f;
    const auto box = px.rect (2.0f, cy - 3.0f, 6.0f, 6.0f);

    if (getToggleState())
    {
        g.setColour (Palette::accent);
        g.fillRect (box);
    }
    else
    {
        Draw::outline (g, px, box, Palette::ink3);
    }

    Draw::text (g, getName(), 11.0f, cy, Fonts::Role::label, Palette::ink);
}

void PillButton::paintButton (juce::Graphics& g, bool, bool)
{
    const auto r = getLocalBounds().toFloat();
    Draw::pill (g, r, Palette::off);

    const bool dim = isDimmed != nullptr && isDimmed();
    Draw::text (g, getName(), r.getCentreX(), r.getCentreY(), Fonts::Role::label,
                dim ? Palette::dText : getToggleState() ? Palette::accent : Palette::ink, juce::Justification::centred);
}

//==============================================================================
SegmentPill::SegmentPill (juce::StringArray segments) : names (std::move (segments))
{
    float total = 0.0f;
    for (auto& n : names)
    {
        widths.push_back (std::round (Draw::width (n, Fonts::Role::label)) + 16.0f);
        total += widths.back();
    }
    idealWidth = (int) std::ceil (total);
    setMouseCursor (juce::MouseCursor::PointingHandCursor);
}

void SegmentPill::setSelected (int i, bool notify)
{
    i = juce::jlimit (0, names.size() - 1, i);
    if (i == selected)
        return;

    selected = i;
    repaint();
    if (notify && onChange)
        onChange (selected);
}

void SegmentPill::paint (juce::Graphics& g)
{
    const auto r = getLocalBounds().toFloat();
    Draw::pill (g, r, Palette::off);

    float x = 0.0f;
    for (int i = 0; i < names.size(); ++i)
    {
        const float w = widths[(size_t) i];
        if (i == selected)
            Draw::pill (g, { x + 1.0f, 1.0f, w - 2.0f, r.getHeight() - 2.0f }, Palette::plot, Palette::rule);
        Draw::text (g, names[i], x + w * 0.5f, r.getCentreY(), Fonts::Role::label,
                    i == selected ? Palette::ink : Palette::ink2, juce::Justification::centred);
        x += w;
    }
}

void SegmentPill::mouseDown (const juce::MouseEvent& e)
{
    float x = 0.0f;
    for (int i = 0; i < names.size(); ++i)
    {
        x += widths[(size_t) i];
        if (e.position.x < x)
        {
            setSelected (i, true);
            return;
        }
    }
}
