#include "MBCLookAndFeel.h"

namespace
{
    // the platform's standard sans
   #if JUCE_MAC
    constexpr const char* uiTypeface = "Helvetica Neue";
   #else
    constexpr const char* uiTypeface = "Segoe UI";
   #endif
}

juce::Font Fonts::ui (float height)
{
    return juce::Font (juce::FontOptions (uiTypeface, "Regular", height));
}

//==============================================================================
MBCLookAndFeel::MBCLookAndFeel()
{
    // text editors / menus use the default sans: make that the UI face too
    setDefaultSansSerifTypefaceName (uiTypeface);

    setColour (juce::ResizableWindow::backgroundColourId, Palette::bg);
    setColour (juce::PopupMenu::backgroundColourId, Palette::panelHi);
    setColour (juce::PopupMenu::textColourId, Palette::text);
    setColour (juce::PopupMenu::highlightedBackgroundColourId, Palette::text);
    setColour (juce::PopupMenu::highlightedTextColourId, Palette::panelHi);
    setColour (juce::TooltipWindow::textColourId, Palette::text);
    setColour (juce::Label::textColourId, Palette::text);
    setColour (juce::TextEditor::backgroundColourId, Palette::panelHi);
    setColour (juce::TextEditor::textColourId, Palette::text);
    setColour (juce::TextEditor::highlightColourId, Palette::text.withAlpha (0.15f));
    setColour (juce::TextEditor::highlightedTextColourId, Palette::text);
    setColour (juce::TextEditor::outlineColourId, Palette::lineHi);
    setColour (juce::TextEditor::focusedOutlineColourId, Palette::text);
    setColour (juce::CaretComponent::caretColourId, Palette::text);
    setColour (juce::TextButton::buttonOnColourId, Palette::text);
}

void MBCLookAndFeel::drawRotarySlider (juce::Graphics& g, int x, int y, int w, int h, float pos,
                                       float startAngle, float endAngle, juce::Slider&)
{
    // solid disc + pointer, a ring of tiny dots marking the 12 detent steps
    const auto bounds = juce::Rectangle<int> (x, y, w, h).toFloat().reduced (4.0f);
    const float r = juce::jmin (bounds.getWidth(), bounds.getHeight()) * 0.5f;
    const auto  c = bounds.getCentre();
    const float angle = startAngle + pos * (endAngle - startAngle);

    constexpr int steps = 12;
    for (int i = 0; i <= steps; ++i)
    {
        const float a = startAngle + (endAngle - startAngle) * (float) i / (float) steps;
        const bool end = (i == 0 || i == steps);
        g.setColour (end ? Palette::soft : Palette::line.withMultipliedAlpha (2.0f));
        const float d = end ? 1.4f : 1.0f;
        g.fillEllipse (juce::Rectangle<float> (d, d).withCentre (c.getPointOnCircumference (r + 3.0f, a)));
    }

    const auto disc = juce::Rectangle<float> (r * 2.0f, r * 2.0f).withCentre (c);
    g.setColour (Palette::knob);
    g.fillEllipse (disc);

    g.setColour (Palette::accent);
    g.drawLine ({ c.getPointOnCircumference (r * 0.45f, angle), c.getPointOnCircumference (r * 0.88f, angle) },
                juce::jlimit (1.3f, 1.8f, r * 0.07f));
}

void MBCLookAndFeel::drawButtonBackground (juce::Graphics& g, juce::Button& b, const juce::Colour&,
                                           bool, bool)
{
    const auto r = b.getLocalBounds().toFloat().reduced (0.5f);
    const float cs = juce::jmin (3.0f, r.getHeight() * 0.5f);

    // segmented groups: only the outer corners are rounded
    const bool left  = ! b.isConnectedOnLeft();
    const bool right = ! b.isConnectedOnRight();
    juce::Path shape;
    shape.addRoundedRectangle (r.getX(), r.getY(), r.getWidth(), r.getHeight(), cs, cs, left, right, left, right);

    if (b.getToggleState())
    {
        g.setColour (b.findColour (juce::TextButton::buttonOnColourId));
        g.fillPath (shape);
        return;
    }

    g.setColour (Palette::line.withMultipliedAlpha (1.6f));
    g.strokePath (shape, juce::PathStrokeType (Palette::hair));
}

void MBCLookAndFeel::drawButtonText (juce::Graphics& g, juce::TextButton& b, bool, bool)
{
    g.setFont (Fonts::ui (juce::jmin (12.0f, (float) b.getHeight() * 0.62f)));
    g.setColour (b.getToggleState() ? Palette::panelHi : Palette::text);
    g.drawText (b.getButtonText(), b.getLocalBounds(), juce::Justification::centred, false);
}

void MBCLookAndFeel::drawPopupMenuBackground (juce::Graphics& g, int w, int h)
{
    g.fillAll (Palette::panelHi);
    g.setColour (Palette::lineHi);
    g.drawRect (juce::Rectangle<float> ((float) w, (float) h), Palette::hair);
}

void MBCLookAndFeel::drawTooltip (juce::Graphics& g, const juce::String& text, int w, int h)
{
    g.fillAll (Palette::panelHi);
    g.setColour (Palette::lineHi);
    g.drawRect (juce::Rectangle<float> ((float) w, (float) h), Palette::hair);
    g.setColour (Palette::text);
    g.setFont (Fonts::ui (12.5f));
    g.drawFittedText (text.toLowerCase(), juce::Rectangle<int> (w, h).reduced (8, 2), juce::Justification::centredLeft, 4);
}

juce::Rectangle<int> MBCLookAndFeel::getTooltipBounds (const juce::String& text, juce::Point<int> pos, juce::Rectangle<int> parent)
{
    juce::GlyphArrangement ga;
    ga.addLineOfText (Fonts::ui (12.5f), text.toLowerCase(), 0.0f, 0.0f);
    const int w = juce::roundToInt (ga.getBoundingBox (0, -1, true).getWidth()) + 18;
    const int h = 22;
    return juce::Rectangle<int> (pos.x > parent.getCentreX() ? pos.x - (w + 12) : pos.x + 18,
                                 pos.y > parent.getCentreY() ? pos.y - (h + 6) : pos.y + 6, w, h)
               .constrainedWithin (parent);
}

void MBCLookAndFeel::drawLabel (juce::Graphics& g, juce::Label& l)
{
    if (! l.isBeingEdited())
    {
        g.setColour (l.findColour (juce::Label::textColourId));
        g.setFont (l.getFont());
        g.drawFittedText (l.getText(), l.getLocalBounds(), l.getJustificationType(), 1);
    }
}

void MBCLookAndFeel::fillTextEditorBackground (juce::Graphics& g, int w, int h, juce::TextEditor&)
{
    g.setColour (Palette::panelHi);
    g.fillRect (0, 0, w, h);
}

void MBCLookAndFeel::drawTextEditorOutline (juce::Graphics& g, int w, int h, juce::TextEditor&)
{
    g.setColour (Palette::text);
    g.drawRect (juce::Rectangle<float> ((float) w, (float) h), Palette::hair);
}
