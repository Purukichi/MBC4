#pragma once

#include <juce_gui_basics/juce_gui_basics.h>

/** Quiet monochrome: a slightly grey base, near-black ink for lines and text,
    hairlines derived from the ink. */
namespace Palette
{
    const juce::Colour bg        { 0xffecebe9 };   // window base (not pure white)
    const juce::Colour panel     { 0xfff3f3f1 };   // strips / side panel
    const juce::Colour panelHi   { 0xfff8f8f7 };   // selected strip
    const juce::Colour plot      { 0xfff6f6f4 };   // graph area

    const juce::Colour text      { 0xff2b2b2a };   // ink (not pure black)
    const juce::Colour line      = text.withAlpha (0.12f);
    const juce::Colour lineHi    = text.withAlpha (0.28f);
    const juce::Colour soft      = text.withAlpha (0.50f);   // axis labels, secondary marks

    const juce::Colour knob      { 0xff4a4947 };   // knob discs: a softer, greyer black
    const juce::Colour accent    { 0xff39ff14 };   // the one accent (fluorescent green): knob pointers
    const juce::Colour white     { 0xffffffff };

    constexpr float hair = 0.45f;                  // stroke width of hairlines
    constexpr float radius = 6.0f;                 // corner radius of every block

    /** soft drop shadow used instead of borders around blocks */
    inline juce::DropShadow blockShadow() { return { juce::Colour (0x0f000000), 18, { 0, 4 } }; }
    inline juce::DropShadow blockContact() { return { juce::Colour (0x0a000000), 3, { 0, 1 } }; }
}

namespace Fonts
{
    /** the one UI typeface and weight (standard Windows sans, regular) */
    juce::Font ui (float height);
}

//==============================================================================
class MBCLookAndFeel : public juce::LookAndFeel_V4
{
public:
    MBCLookAndFeel();

    void drawRotarySlider (juce::Graphics&, int x, int y, int w, int h, float pos,
                           float startAngle, float endAngle, juce::Slider&) override;

    void drawButtonBackground (juce::Graphics&, juce::Button&, const juce::Colour&,
                               bool highlighted, bool down) override;
    void drawButtonText (juce::Graphics&, juce::TextButton&, bool highlighted, bool down) override;

    juce::Font getPopupMenuFont() override { return Fonts::ui (14.0f); }
    void drawPopupMenuBackground (juce::Graphics&, int w, int h) override;

    void drawTooltip (juce::Graphics&, const juce::String& text, int w, int h) override;
    juce::Rectangle<int> getTooltipBounds (const juce::String& text, juce::Point<int> pos, juce::Rectangle<int> parent) override;

    void drawLabel (juce::Graphics&, juce::Label&) override;
    void fillTextEditorBackground (juce::Graphics&, int w, int h, juce::TextEditor&) override;
    void drawTextEditorOutline (juce::Graphics&, int w, int h, juce::TextEditor&) override;
};
