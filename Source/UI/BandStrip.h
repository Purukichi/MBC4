#pragma once

#include <juce_audio_processors/juce_audio_processors.h>
#include "PluginProcessor.h"

//==============================================================================
/** Rotary slider with relative dragging: up/right raises the value.
    Plain drags move in 12 detented steps (like a stepped analogue pot); Ctrl (or Shift) drags
    continuously and 6x finer.
    Alt+click returns to the default, double-click asks the owner to open text entry. */
class FineSlider : public juce::Slider
{
public:
    std::function<void()> onDoubleClick, onAltClick;
    static constexpr int detents = 12;   // steps across the full range (13 positions, centre included)

    void mouseDown (const juce::MouseEvent&) override;
    void mouseDrag (const juce::MouseEvent&) override;
    void mouseUp (const juce::MouseEvent&) override;
    void mouseDoubleClick (const juce::MouseEvent&) override;

private:
    juce::Point<float> lastPos;
    double dragProportion = 0.0;
    bool altClick = false;
};

//==============================================================================
/** Minimal knob: disc + short name underneath. The value replaces the name while
    hovering or dragging, and a double-click opens text entry in the same place. */
class Knob : public juce::Component
{
public:
    Knob (juce::AudioProcessorValueTreeState&, const juce::String& paramId, const juce::String& name);
    ~Knob() override;

    void paint (juce::Graphics&) override;
    void resized() override;
    void mouseEnter (const juce::MouseEvent&) override { updateHover(); }
    void mouseExit (const juce::MouseEvent&) override  { updateHover(); }
    void mouseUp (const juce::MouseEvent&) override    { updateHover(); }

    void setMaxDiameter (int d) { maxDiameter = d; resized(); }

private:
    FineSlider slider;
    juce::Label value;
    juce::String name;
    int maxDiameter = 1000;
    std::unique_ptr<juce::AudioProcessorValueTreeState::SliderAttachment> attachment;

    void refreshText();
    void updateHover();

    JUCE_DECLARE_NON_COPYABLE_WITH_LEAK_DETECTOR (Knob)
};

//==============================================================================
/** All controls of one band + its gain-reduction meter. */
class BandStrip : public juce::Component
{
public:
    BandStrip (MBC4Processor&, int bandIndex);

    void setSelected (bool);
    void updateMeter();   // from the editor timer

    std::function<void (int)> onSelect;

    void paint (juce::Graphics&) override;
    void resized() override;
    void mouseDown (const juce::MouseEvent&) override;

private:
    MBC4Processor& proc;
    const int band;
    bool selected = false;

    Knob thresh, ratio, knee, attack, release, range, mix, gain;
    juce::TextButton solo { "s" }, mute { "m" }, bypass { "b" }, learn { "match" };
    std::unique_ptr<juce::AudioProcessorValueTreeState::ButtonAttachment> soloAtt, muteAtt, bypassAtt;

    // band edges, double-click to type
    juce::Label loFreq, hiFreq;

    juce::Rectangle<int> header, letterArea, freqRow, meterArea;
    float grShown = 0.0f, grPeak = 0.0f;
    int peakHold = 0;
    float lastLo = -1.0f, lastHi = -1.0f;

    static constexpr double learnSeconds = 3.0;
    void toggleLearn();
    void updateLearn();

    void setCrossoverFromText (int index, const juce::String& text);
    void layoutFreqRow();
    std::array<Knob*, 8> knobs() { return { &thresh, &ratio, &knee, &attack, &release, &range, &mix, &gain }; }

    JUCE_DECLARE_NON_COPYABLE_WITH_LEAK_DETECTOR (BandStrip)
};
