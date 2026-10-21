#include "MBCLookAndFeel.h"
#include "Grid.h"
#include "BinaryData.h"

namespace
{
    juce::Typeface::Ptr typeface (bool light)
    {
        static auto lightFace   = juce::Typeface::createSystemTypefaceFor (BinaryData::ChivoMonoLight_ttf,   (size_t) BinaryData::ChivoMonoLight_ttfSize);
        static auto regularFace = juce::Typeface::createSystemTypefaceFor (BinaryData::ChivoMonoRegular_ttf, (size_t) BinaryData::ChivoMonoRegular_ttfSize);
        return light ? lightFace : regularFace;
    }

   #if JUCE_MAC
    constexpr const char* japaneseFace = "Hiragino Sans";
   #else
    constexpr const char* japaneseFace = "Yu Gothic UI";
   #endif
}

//==============================================================================
juce::Font Fonts::mono (float size, bool light, float trackingPx)
{
    juce::Font f (juce::FontOptions (typeface (light)).withPointHeight (size));
    if (trackingPx != 0.0f)
        f.setExtraKerningFactor (trackingPx / f.getHeight());
    return f;
}

juce::Font Fonts::of (Role r)
{
    switch (r)
    {
        case Role::label: return mono (8.0f, false, 0.8f);
        case Role::value: return mono (9.0f, true);
        case Role::note:  return mono (7.5f, true, 0.6f);
        case Role::tick:  return mono (7.5f, true);
    }
    return mono (9.0f, true);
}

juce::Font Fonts::japanese (float size)
{
    return juce::Font (juce::FontOptions (japaneseFace, size, juce::Font::plain));
}

//==============================================================================
namespace
{
    juce::Point<int> rootOffset (const juce::Component& c)
    {
        // position inside the transformed main view (whose own scale is in the graphics context)
        juce::Point<int> o;
        for (auto* p = &c; p != nullptr && ! p->isTransformed(); p = p->getParentComponent())
            o += p->getPosition();
        return o;
    }
}

Draw::Px::Px (juce::Graphics& g, const juce::Component& c)
    : k (juce::jmax (0.01f, g.getInternalContext().getPhysicalPixelScaleFactor()))
{
    const auto o = rootOffset (c);
    ox = (float) o.x;
    oy = (float) o.y;
}

juce::Rectangle<float> Draw::Px::rect (float x, float y, float w, float h) const noexcept
{
    const float x0 = snapX (x), y0 = snapY (y);
    return { x0, y0, juce::jmax (1.0f / k, snapX (x + w) - x0), juce::jmax (1.0f / k, snapY (y + h) - y0) };
}

juce::Rectangle<float> Draw::Px::cell (float x, float y) const noexcept
{
    return rect (x, y, Grid::cell, Grid::cell);
}

//==============================================================================
void Draw::text (juce::Graphics& g, const juce::String& s, float x, float y, Role r, juce::Colour c, juce::Justification j)
{
    const auto f = Fonts::of (r);
    const float h = f.getHeight() + 4.0f;
    const float w = 400.0f;
    g.setFont (f);
    g.setColour (c);

    juce::Rectangle<float> area;
    if (j.testFlags (juce::Justification::right))                 area = { x - w, y - h * 0.5f, w, h };
    else if (j.testFlags (juce::Justification::horizontallyCentred)) area = { x - w * 0.5f, y - h * 0.5f, w, h };
    else                                                          area = { x, y - h * 0.5f, w, h };

    g.drawText (s, area, j.getOnlyHorizontalFlags() | juce::Justification::verticallyCentred, false);
}

float Draw::width (const juce::String& s, Role r)
{
    return juce::GlyphArrangement::getStringWidth (Fonts::of (r), s);
}

void Draw::ruleH (juce::Graphics& g, float x1, float x2, float y)
{
    g.setColour (Palette::rule);
    g.fillRect (juce::Rectangle<float> (x1, y - 0.1f, x2 - x1, 0.2f));
}

void Draw::ruleV (juce::Graphics& g, float x, float y1, float y2)
{
    g.setColour (Palette::rule);
    g.fillRect (juce::Rectangle<float> (x - 0.1f, y1, 0.2f, y2 - y1));
}

void Draw::lineH (juce::Graphics& g, const Px& px, float x1, float x2, float y, juce::Colour c, float t)
{
    const float d = px.dev (t);
    g.setColour (c);
    g.fillRect (juce::Rectangle<float> (px.snapX (x1), px.snapY (y - d * 0.5f), px.snapX (x2) - px.snapX (x1), d));
}

void Draw::lineV (juce::Graphics& g, const Px& px, float x, float y1, float y2, juce::Colour c, float t)
{
    const float d = px.dev (t);
    g.setColour (c);
    g.fillRect (juce::Rectangle<float> (px.snapX (x - d * 0.5f), px.snapY (y1), d, px.snapY (y2) - px.snapY (y1)));
}

void Draw::outline (juce::Graphics& g, const Px& px, juce::Rectangle<float> r, juce::Colour c)
{
    const float d = 1.0f / px.k;
    const auto s = px.rect (r.getX(), r.getY(), r.getWidth(), r.getHeight());
    g.setColour (c);
    g.fillRect (s.withHeight (d));
    g.fillRect (s.withTop (s.getBottom() - d));
    g.fillRect (s.withWidth (d));
    g.fillRect (s.withLeft (s.getRight() - d));
}

void Draw::pill (juce::Graphics& g, juce::Rectangle<float> r, juce::Colour fill, juce::Colour stroke)
{
    const float rad = r.getHeight() * 0.5f;
    g.setColour (fill);
    g.fillRoundedRectangle (r, rad);
    if (! stroke.isTransparent())
    {
        const float k = juce::jmax (0.01f, g.getInternalContext().getPhysicalPixelScaleFactor());
        g.setColour (stroke);
        g.drawRoundedRectangle (r.reduced (0.5f / k), rad, 1.0f / k);
    }
}

void Draw::bubble (juce::Graphics& g, const juce::String& s, float cx, float cy, float minX, float maxX)
{
    const float w = width (s, Role::value) + 10.0f, h = 14.0f;
    const float x = juce::jlimit (minX, juce::jmax (minX, maxX - w), cx - w * 0.5f);
    g.setColour (Palette::ink);
    g.fillRoundedRectangle ({ x, cy - h * 0.5f, w, h }, 3.0f);
    text (g, s, x + w * 0.5f, cy, Role::value, Palette::plot, juce::Justification::centred);
}

void Draw::sectionHeader (juce::Graphics& g, const juce::String& number, const juce::String& name, float x, float y, juce::Colour c)
{
    text (g, number, x, y, Role::note, Palette::ink3);
    text (g, name, x + 14.0f, y, Role::label, c);
}

namespace
{
    using Glyph = std::array<const char*, 7>;

    const Glyph* digitGlyph (juce::juce_wchar ch)
    {
        static const Glyph digits[10] = {
            { "01110","10001","10001","10001","10001","10001","01110" },
            { "00100","01100","00100","00100","00100","00100","01110" },
            { "01110","10001","00001","00010","00100","01000","11111" },
            { "11111","00010","00100","00010","00001","10001","01110" },
            { "00010","00110","01010","10010","11111","00010","00010" },
            { "11111","10000","11110","00001","00001","10001","01110" },
            { "00110","01000","10000","11110","10001","10001","01110" },
            { "11111","00001","00010","00100","01000","01000","01000" },
            { "01110","10001","10001","01110","10001","10001","01110" },
            { "01110","10001","10001","01111","00001","00010","01100" } };
        static const Glyph minus { "00000","00000","00000","11111","00000","00000","00000" };
        static const Glyph plus  { "00000","00100","00100","11111","00100","00100","00000" };
        static const Glyph dot   { "0","0","0","0","0","0","1" };

        if (ch >= '0' && ch <= '9') return &digits[ch - '0'];
        if (ch == '-') return &minus;
        if (ch == '+') return &plus;
        if (ch == '.') return &dot;
        return nullptr;
    }
}

void Draw::dotNumber (juce::Graphics& g, const Px& px, const juce::String& s, float cx, float top, juce::Colour c)
{
    constexpr float P = 2.0f, D = 1.6f;
    auto adv = [] (const Glyph& gl) { return (float) std::strlen (gl[0]) * P + P; };

    float w = -P;
    for (auto ch : s)
        if (auto* gl = digitGlyph (ch))
            w += adv (*gl);

    juce::RectangleList<float> on, offDots;
    float x = cx - w * 0.5f;
    for (auto ch : s)
    {
        auto* gl = digitGlyph (ch);
        if (gl == nullptr)
            continue;
        for (int r = 0; r < 7; ++r)
            for (int col = 0; (*gl)[(size_t) r][col] != 0; ++col)
                ((*gl)[(size_t) r][col] == '1' ? on : offDots).addWithoutMerging (px.rect (x + (float) col * P, top + (float) r * P, D, D));
        x += adv (*gl);
    }
    g.setColour (Palette::off);
    g.fillRectList (offDots);
    g.setColour (c);
    g.fillRectList (on);
}

void Draw::logo (juce::Graphics& g, const Px& px, float x, float y)
{
    static const std::array<const char*, 5> letters[4] = {
        { "1001","1111","1111","1001","1001" },    // M
        { "1110","1001","1110","1001","1110" },    // B
        { "0111","1000","1000","1000","0111" },    // C
        { "1010","1010","1111","0010","0010" } };  // 4

    juce::RectangleList<float> ink, acc;
    for (int n = 0; n < 4; ++n)
        for (int r = 0; r < 5; ++r)
            for (int c = 0; c < 4; ++c)
                if (letters[n][(size_t) r][c] == '1')
                    (n == 3 ? acc : ink).addWithoutMerging (px.cell (x + (float) (n * 5 + c) * Grid::pitch, y + (float) r * Grid::pitch));
    g.setColour (Palette::ink);
    g.fillRectList (ink);
    g.setColour (Palette::accent);
    g.fillRectList (acc);
}

void Draw::cells (juce::Graphics& g, const Px& px, float x, float y, int cols, int rows, juce::Colour c)
{
    juce::RectangleList<float> list;
    for (int r = 0; r < rows; ++r)
        for (int col = 0; col < cols; ++col)
            list.addWithoutMerging (px.cell (x + (float) col * Grid::pitch, y + (float) r * Grid::pitch));
    g.setColour (c);
    g.fillRectList (list);
}

//==============================================================================
MBCLookAndFeel::MBCLookAndFeel()
{
    setColour (juce::ResizableWindow::backgroundColourId, Palette::cell);
    setColour (juce::PopupMenu::backgroundColourId, Palette::cell);
    setColour (juce::PopupMenu::textColourId, Palette::ink);
    setColour (juce::PopupMenu::highlightedBackgroundColourId, Palette::ink);
    setColour (juce::PopupMenu::highlightedTextColourId, Palette::plot);
    setColour (juce::TextEditor::backgroundColourId, Palette::plot);
    setColour (juce::TextEditor::textColourId, Palette::ink);
    setColour (juce::TextEditor::highlightColourId, Palette::accent.withAlpha (0.25f));
    setColour (juce::TextEditor::highlightedTextColourId, Palette::ink);
    setColour (juce::TextEditor::outlineColourId, Palette::rule);
    setColour (juce::TextEditor::focusedOutlineColourId, Palette::rule);
    setColour (juce::CaretComponent::caretColourId, Palette::ink);
}

void MBCLookAndFeel::drawPopupMenuBackground (juce::Graphics& g, int w, int h)
{
    g.fillAll (Palette::cell);
    g.setColour (Palette::rule);
    g.drawRect (juce::Rectangle<float> ((float) w, (float) h), 1.0f);
}

juce::TextLayout MBCLookAndFeel::tooltipLayout (const juce::String& text, float maxWidth) const
{
    const auto heading = text.upToFirstOccurrenceOf ("\n", false, false);
    const auto body    = text.fromFirstOccurrenceOf ("\n", false, false);

    juce::AttributedString s;
    s.append (heading.toUpperCase(), Fonts::mono (8.0f * uiScale, false, 0.8f * uiScale), Palette::accent);
    if (body.isNotEmpty())
    {
        s.append ("\n", Fonts::mono (4.0f * uiScale, false), Palette::accent);   // small gap below the heading
        s.append ("\n" + body, Fonts::japanese (10.5f * uiScale), Palette::plot);
    }
    s.setLineSpacing (2.0f * uiScale);
    s.setWordWrap (juce::AttributedString::byChar);

    juce::TextLayout layout;
    layout.createLayout (s, maxWidth);
    return layout;
}

void MBCLookAndFeel::drawTooltip (juce::Graphics& g, const juce::String& text, int w, int h)
{
    const auto r = juce::Rectangle<float> ((float) w, (float) h);
    g.setColour (Palette::ink.withAlpha (0.96f));
    g.fillRoundedRectangle (r, 3.0f * uiScale);

    const float padX = 9.0f * uiScale, padY = 7.0f * uiScale;
    tooltipLayout (text, (float) w - 2.0f * padX + 1.0f).draw (g, r.reduced (padX, padY));
}

juce::Rectangle<int> MBCLookAndFeel::getTooltipBounds (const juce::String& text, juce::Point<int> pos, juce::Rectangle<int> parent)
{
    const float padX = 9.0f * uiScale, padY = 7.0f * uiScale;
    const auto layout = tooltipLayout (text, 220.0f * uiScale);

    float textW = 0.0f;
    for (auto& line : layout)
        textW = juce::jmax (textW, line.getLineBoundsX().getEnd());

    // generous margins so nothing is cut off
    const int w = (int) std::ceil (textW + 2.0f * padX) + 2;
    const int h = (int) std::ceil (layout.getHeight() + 2.0f * padY) + 2;
    const int gap = juce::roundToInt (14.0f * uiScale);

    return juce::Rectangle<int> (pos.x + w + gap > parent.getRight() ? pos.x - w - gap / 2 : pos.x + gap,
                                 pos.y + h + gap > parent.getBottom() ? pos.y - h - gap / 2 : pos.y + gap, w, h)
               .constrainedWithin (parent);
}

void MBCLookAndFeel::drawCornerResizer (juce::Graphics& g, int w, int h, bool over, bool dragging)
{
    const float p = Grid::pitch * uiScale, c = Grid::cell * uiScale;
    const float right = (float) w - 2.0f * uiScale, bottom = (float) h - 2.0f * uiScale;
    g.setColour (over || dragging ? Palette::ink2 : Palette::ink3);
    for (int r = 0; r < 3; ++r)
        for (int col = 2 - r; col < 3; ++col)
            g.fillRect (juce::Rectangle<float> (std::round (right - (float) (3 - col) * p + (p - c)),
                                                std::round (bottom - (float) (3 - r) * p + (p - c)), c, c));
}

void MBCLookAndFeel::fillTextEditorBackground (juce::Graphics& g, int w, int h, juce::TextEditor&)
{
    g.setColour (Palette::plot);
    g.fillRect (0, 0, w, h);
}

void MBCLookAndFeel::drawTextEditorOutline (juce::Graphics& g, int w, int h, juce::TextEditor&)
{
    g.setColour (Palette::rule);
    g.drawRect (juce::Rectangle<float> ((float) w, (float) h), 1.0f);
}
