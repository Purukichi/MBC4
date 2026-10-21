#pragma once

#include <juce_audio_processors/juce_audio_processors.h>

/** How a fader moves: plain drags and the wheel stop on fixed steps (or on a list of
    values), Shift or Ctrl moves finely. */
struct Steps
{
    float step = 1.0f;
    float fine = 0.1f;
    std::vector<float> list;   // when set, plain moves stop only on these values
};

enum class Format { dB, ms, percent, ratio };

/** "-24", "2.5" (one decimal at most, never "-0") */
juce::String numberText (float v);
juce::String formatValue (float v, Format);

//==============================================================================
/** Fader drawn with cells: the cells from the base value to the current value are lit,
    the value's own cell is black with a thin needle through it. Name and value text are
    part of the fader.
    Drag / wheel: steps (Shift or Ctrl: fine). Double-click: default.
    Double-click the value text: type a value (Enter confirms, Esc cancels). */
class CellFader : public juce::Component,
                  public juce::SettableTooltipClient
{
public:
    enum class Orientation { vertical, horizontal };

    CellFader (juce::AudioProcessorValueTreeState&, const juce::String& paramId, const juce::String& label,
               Orientation, Steps, Format);

    std::function<bool()> isDimmed;   // e.g. the band is bypassed

    void paint (juce::Graphics&) override;
    void resized() override;
    bool hitTest (int x, int y) override;

    void mouseDown (const juce::MouseEvent&) override;
    void mouseDrag (const juce::MouseEvent&) override;
    void mouseUp (const juce::MouseEvent&) override;
    void mouseDoubleClick (const juce::MouseEvent&) override;
    void mouseWheelMove (const juce::MouseEvent&, const juce::MouseWheelDetails&) override;

private:
    juce::RangedAudioParameter& param;
    const juce::String label;
    const Orientation orientation;
    const Steps steps;
    const Format format;
    juce::ParameterAttachment attachment;
    float value = 0.0f;

    // drag state: the anchor moves whenever the fine modifier changes
    juce::Point<float> anchor;
    float anchorNorm = 0.0f;
    bool dragFine = false, dragging = false;

    juce::TextEditor editor;

    bool vertical() const noexcept { return orientation == Orientation::vertical; }
    int  numCells() const noexcept;
    juce::Rectangle<float> valueArea() const;
    float snap (float v, bool fine) const;
    void  setFromUser (float v);
    void  openEditor();
    void  closeEditor (bool commit);

    JUCE_DECLARE_NON_COPYABLE_WITH_LEAK_DETECTOR (CellFader)
};

//==============================================================================
/** 6 px square toggle with its name: orange when on, a thin frame when off. */
class SquareToggle : public juce::Button
{
public:
    explicit SquareToggle (const juce::String& name) : juce::Button (name) { setClickingTogglesState (true); }
    void paintButton (juce::Graphics&, bool highlighted, bool down) override;
};

/** Small push button in a rounded groove; the text turns orange while it is active. */
class PillButton : public juce::Button
{
public:
    explicit PillButton (const juce::String& name) : juce::Button (name) {}
    std::function<bool()> isDimmed;
    void paintButton (juce::Graphics&, bool highlighted, bool down) override;
};

/** Segmented selector: a rounded groove, the selected segment raised and framed. */
class SegmentPill : public juce::Component,
                    public juce::SettableTooltipClient
{
public:
    explicit SegmentPill (juce::StringArray segments);

    void setSelected (int i, bool notify);
    int  getSelected() const noexcept { return selected; }
    std::function<void (int)> onChange;

    int getIdealWidth() const noexcept { return idealWidth; }

    void paint (juce::Graphics&) override;
    void mouseDown (const juce::MouseEvent&) override;

private:
    juce::StringArray names;
    std::vector<float> widths;
    int idealWidth = 0, selected = 0;
};
