#pragma once

#include <juce_audio_processors/juce_audio_processors.h>
#include "PluginProcessor.h"
#include "UI/MBCLookAndFeel.h"
#include "UI/GraphView.h"
#include "UI/BandStrip.h"

//==============================================================================
/** The whole UI, laid out at a fixed base size; the editor scales it. */
class MainView : public juce::Component,
                 private juce::Timer
{
public:
    static constexpr int baseW = 1240, baseH = 800;   // = Grid::baseW / baseH

    explicit MainView (MBC4Processor&);
    ~MainView() override;

    void paint (juce::Graphics&) override;
    void resized() override;

private:
    void timerCallback() override;
    void selectBand (int b);
    void paintMeters (juce::Graphics&);
    void renderShadows();

    MBC4Processor& proc;

    GraphView graph;
    std::array<std::unique_ptr<BandStrip>, MBC4Processor::numBands> strips;

    Knob inGain, mix, outGain, look;

    juce::TextButton anBoth { "in + out" }, anPost { "out" }, anPre { "in" }, anOff { "off" };

    juce::Rectangle<int> topBar, sidePanel, meterArea;
    juce::Rectangle<float> meterPlot;   // the graph's plot rows, in the side panel
    juce::Image shadows;                 // block drop shadows, cached per layout
    float inShown = -100.0f, outShown = -100.0f;
    float inHold = -100.0f, outHold = -100.0f;
    int inHoldTime = 0, outHoldTime = 0;
    int selected = 0;

    JUCE_DECLARE_NON_COPYABLE_WITH_LEAK_DETECTOR (MainView)
};

//==============================================================================
class MBC4Editor : public juce::AudioProcessorEditor
{
public:
    explicit MBC4Editor (MBC4Processor&);
    ~MBC4Editor() override;

    void resized() override;

private:
    MBC4Processor& proc;
    MBCLookAndFeel laf;
    MainView view;
    juce::TooltipWindow tooltips { this, 600 };
    bool restoring = true;   // ignore the resizes made while the constructor restores the size

    JUCE_DECLARE_NON_COPYABLE_WITH_LEAK_DETECTOR (MBC4Editor)
};
