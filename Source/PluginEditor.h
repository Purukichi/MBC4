#pragma once

#include <juce_audio_processors/juce_audio_processors.h>
#include "PluginProcessor.h"
#include "UI/MBCLookAndFeel.h"
#include "UI/GraphView.h"
#include "UI/BandStrip.h"

//==============================================================================
/** The whole UI, laid out at a fixed base size; the editor scales it.
    01 INPUT | 02 SPLIT over 03 - 06 BAND A - D | 07 OUTPUT */
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
    struct Meter
    {
        float shown = -100.0f, hold = -100.0f;
        int holdTime = 0;
        void update (float v);
    };

    void timerCallback() override;
    void paintMeter (juce::Graphics&, const Draw::Px&, float cx, const Meter&);

    MBC4Processor& proc;

    GraphView graph;
    std::array<std::unique_ptr<BandStrip>, MBC4Processor::numBands> strips;
    CellFader inGain, look, mix, outGain;

    Meter inMeter, outMeter;

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
    juce::TooltipWindow tooltips { this, 500 };
    bool restoring = true;   // ignore the resizes made while the constructor restores the size

    JUCE_DECLARE_NON_COPYABLE_WITH_LEAK_DETECTOR (MBC4Editor)
};
