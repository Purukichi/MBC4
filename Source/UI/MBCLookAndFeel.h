#pragma once

#include <juce_gui_basics/juce_gui_basics.h>

/** PC1 style: one background colour, 0.2 px rules, near-black ink in three steps,
    square cells for every mark, signal orange as the only accent (on / active / cut). */
namespace Palette
{
    const juce::Colour cell    { 0xffe8e9e5 };   // the one background
    const juce::Colour rule    { 0xffc3c4bf };   // 0.2 px rules
    const juce::Colour ink     { 0xff111210 };   // text, fader caps, reference lines
    const juce::Colour ink2    { 0xff575854 };   // secondary text, scale numbers
    const juce::Colour ink3    { 0xff8f908b };   // notes, off toggles
    const juce::Colour off     { 0xffdcddd8 };   // unlit cells
    const juce::Colour lit     { 0xffb3b4af };   // lit cells (fader amounts)
    const juce::Colour plot    { 0xfff4f4f1 };   // selected segment, text on dark
    const juce::Colour accent  { 0xffff4d0f };   // signal orange
    const juce::Colour ghost   { 0xffd3d4cf };   // input spectrum behind the output

    // disabled (bypassed band)
    const juce::Colour dLit    { 0xffcbccc7 };
    const juce::Colour dKnob   { 0xffa8a9a4 };
    const juce::Colour dText   { 0xffb0b1ac };

    /** data intensity, weak -> strong */
    const std::array<juce::Colour, 6> ramp { juce::Colour (0xffc7c8c3), juce::Colour (0xffb9bab5), juce::Colour (0xffa6a7a2),
                                             juce::Colour (0xff8f908b), juce::Colour (0xff5c5d59), juce::Colour (0xff2a2b29) };

    /** ramp colour for row j of n (darker towards the top) */
    inline juce::Colour rampAt (int j, int n)
    {
        return ramp[(size_t) juce::jlimit (0, (int) ramp.size() - 1, (int) ((float) j / (float) n * (float) ramp.size() * 1.15f))];
    }
}

namespace Fonts
{
    /** the four type roles (Chivo Mono, embedded) */
    enum class Role { label, value, note, tick };

    juce::Font mono (float size, bool light, float trackingPx = 0.0f);
    juce::Font of (Role);
    juce::Font japanese (float size);
}

namespace Draw
{
    using Role = Fonts::Role;

    /** device-pixel snapping inside a component of the scaled main view */
    struct Px
    {
        Px (juce::Graphics&, const juce::Component&);

        float snapX (float x) const noexcept { return std::round ((x + ox) * k) / k - ox; }
        float snapY (float y) const noexcept { return std::round ((y + oy) * k) / k - oy; }
        juce::Rectangle<float> rect (float x, float y, float w, float h) const noexcept;
        float dev (float t) const noexcept { return juce::jmax (1.0f, std::round (t * k)) / k; }   // >= 1 device px

        float k = 1.0f, ox = 0.0f, oy = 0.0f;
    };

    /** Cells on a whole-device-pixel pitch, so every cell and every gap has the same size
        at any UI scale. The pitch / cell size given in base units are rounded once; the
        cell count that fits a length can therefore differ slightly from scale to scale. */
    struct Lattice
    {
        Lattice (const Px&, float x, float y, float pitch, float cellSize);

        /** cells [i, i+ni) x [j, j+nj), including the gaps between them (base units) */
        juce::Rectangle<float> span (int i, int j, int ni = 1, int nj = 1) const noexcept;
        juce::Rectangle<float> at (int i, int j) const noexcept { return span (i, j); }

        float x (int i) const noexcept { return ((float) (dx0 + i * P)) / k - ox; }   // left edge of column i
        float y (int j) const noexcept { return ((float) (dy0 + j * P)) / k - oy; }   // top edge of row j
        float cellSize() const noexcept { return (float) C / k; }
        float gapSize() const noexcept  { return (float) gap / k; }
        float extent (int n) const noexcept { return (float) (n * P - gap) / k; }      // length of n cells
        int   fit (float length) const noexcept;                                     // cells that fit a length

        float k, ox, oy;
        int P, C, gap, dx0, dy0;
    };

    /** y is the text's centre line */
    void text (juce::Graphics&, const juce::String&, float x, float y, Role, juce::Colour,
               juce::Justification = juce::Justification::left);
    float width (const juce::String&, Role);

    void ruleH (juce::Graphics&, float x1, float x2, float y);
    void ruleV (juce::Graphics&, float x, float y1, float y2);
    void lineH (juce::Graphics&, const Px&, float x1, float x2, float y, juce::Colour, float t = 0.5f);
    void lineV (juce::Graphics&, const Px&, float x, float y1, float y2, juce::Colour, float t = 0.5f);
    void outline (juce::Graphics&, const Px&, juce::Rectangle<float>, juce::Colour);

    void pill (juce::Graphics&, juce::Rectangle<float>, juce::Colour fill, juce::Colour stroke = {});
    void bubble (juce::Graphics&, const juce::String&, float cx, float cy, float minX, float maxX);
    void sectionHeader (juce::Graphics&, const juce::String& number, const juce::String& name, float x, float y,
                        juce::Colour = Palette::ink);

    /** 5 x 7 dot-matrix number, unlit dots drawn faintly */
    void dotNumber (juce::Graphics&, const Px&, const juce::String&, float cx, float top, juce::Colour);
    /** 4 x 5 dot logo, the last letter in the accent */
    void logo (juce::Graphics&, const Px&, float x, float y);
}

//==============================================================================
class MBCLookAndFeel : public juce::LookAndFeel_V4
{
public:
    MBCLookAndFeel();

    /** tooltips live in the unscaled editor: they follow the UI scale through this */
    void setUiScale (float s) noexcept { uiScale = s; }

    juce::Font getPopupMenuFont() override { return Fonts::mono (13.0f, true); }
    void drawPopupMenuBackground (juce::Graphics&, int w, int h) override;

    /** tooltip text: "HEADING\nbody" */
    void drawTooltip (juce::Graphics&, const juce::String& text, int w, int h) override;
    juce::Rectangle<int> getTooltipBounds (const juce::String& text, juce::Point<int> pos, juce::Rectangle<int> parent) override;

    /** the resize corner: a staircase of cells */
    void drawCornerResizer (juce::Graphics&, int w, int h, bool isMouseOver, bool isMouseDragging) override;

    void fillTextEditorBackground (juce::Graphics&, int w, int h, juce::TextEditor&) override;
    void drawTextEditorOutline (juce::Graphics&, int w, int h, juce::TextEditor&) override;

private:
    float uiScale = 1.0f;
    juce::TextLayout tooltipLayout (const juce::String&, float maxWidth) const;
};
