#pragma once

#include <juce_audio_processors/juce_audio_processors.h>
#include "PluginProcessor.h"
#include "Controls.h"

/** Tooltip text: "Heading\n本文" (the body is UTF-8). */
juce::String tooltipText (const char* heading, const char* body);

//==============================================================================
/** One band: header (number, name, range, S / M / B), threshold / range / gain faders,
    gain match, the gain-reduction meter, and ratio / knee / attack / release / mix sliders. */
class BandStrip : public juce::Component
{
public:
    BandStrip (MBC4Processor&, int bandIndex);

    void updateMeter();   // from the editor timer

    void paint (juce::Graphics&) override;
    void resized() override;
    void mouseDoubleClick (const juce::MouseEvent&) override;

private:
    MBC4Processor& proc;
    const int band;

    CellFader thresh, range, gain;
    CellFader ratio, knee, attack, release, mix;
    SquareToggle solo { "S" }, mute { "M" }, bypass { "B" };
    PillButton learn { "MATCH" };
    std::unique_ptr<juce::AudioProcessorValueTreeState::ButtonAttachment> soloAtt, muteAtt, bypassAtt;

    juce::TextEditor freqEditor;   // crossover entry from the header
    int editingCrossover = -1;

    float grShown = 0.0f, grWindowPeak = 0.0f, grReadout = 0.0f;
    int readoutFrames = 0;
    float lastLo = -1.0f, lastHi = -1.0f;
    bool lastBypass = false;

    static constexpr double learnSeconds = 3.0;
    void toggleLearn();
    void updateLearn();

    bool isBypassed() const;
    float edgeHz (int crossover) const;
    juce::String rangeLo() const;
    juce::String rangeHi() const;
    juce::Rectangle<float> loTextArea() const;
    juce::Rectangle<float> hiTextArea() const;
    void editCrossover (int index, juce::Rectangle<float> area);
    void commitCrossover();

    JUCE_DECLARE_NON_COPYABLE_WITH_LEAK_DETECTOR (BandStrip)
};
