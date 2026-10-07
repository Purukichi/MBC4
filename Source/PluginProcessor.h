#pragma once

#include <juce_audio_processors/juce_audio_processors.h>
#include "DSP/MultibandEngine.h"
#include "DSP/AnalyzerFifo.h"

class MBC4Processor : public juce::AudioProcessor
{
public:
    static constexpr int numBands = MultibandEngine::numBands;

    MBC4Processor();
    ~MBC4Processor() override = default;

    void prepareToPlay (double sampleRate, int samplesPerBlock) override;
    void releaseResources() override {}
    bool isBusesLayoutSupported (const BusesLayout& layouts) const override;
    void processBlock (juce::AudioBuffer<float>&, juce::MidiBuffer&) override;
    void processBlockBypassed (juce::AudioBuffer<float>&, juce::MidiBuffer&) override;
    using AudioProcessor::processBlock;
    using AudioProcessor::processBlockBypassed;

    juce::AudioProcessorEditor* createEditor() override;
    bool hasEditor() const override { return true; }

    const juce::String getName() const override { return JucePlugin_Name; }
    bool acceptsMidi() const override  { return false; }
    bool producesMidi() const override { return false; }
    bool isMidiEffect() const override { return false; }
    double getTailLengthSeconds() const override { return 0.0; }

    int getNumPrograms() override { return 1; }
    int getCurrentProgram() override { return 0; }
    void setCurrentProgram (int) override {}
    const juce::String getProgramName (int) override { return {}; }
    void changeProgramName (int, const juce::String&) override {}

    void getStateInformation (juce::MemoryBlock& destData) override;
    void setStateInformation (const void* data, int sizeInBytes) override;

    //==========================================================================
    static juce::String bandParamId (int band, const juce::String& name) { return "b" + juce::String (band + 1) + "_" + name; }
    static juce::String xoverParamId (int index) { return "xover" + juce::String (index + 1); }
    static juce::AudioProcessorValueTreeState::ParameterLayout createLayout();

    juce::AudioProcessorValueTreeState apvts;

    MultibandEngine engine;
    AnalyzerFifo preFifo, postFifo;
    std::atomic<float> inPeakDb { -100.0f }, outPeakDb { -100.0f };

private:
    struct BandPtrs
    {
        std::atomic<float> *thresh, *ratio, *attack, *release, *knee, *gain, *range, *mix, *solo, *mute, *bypass;
    };

    std::array<BandPtrs, numBands> band {};
    std::array<std::atomic<float>*, 3> xover {};
    std::atomic<float> *inGain = nullptr, *outGain = nullptr, *mix = nullptr, *lookahead = nullptr;

    juce::AudioBuffer<float> bypassDelay;
    int bypassWrite = 0;
    int lookaheadSamples() const;
    void updateLookahead();

    juce::SmoothedValue<float> inGainSm, outGainSm;
    juce::HeapBlock<float> mono;
    int monoSize = 0;

    void pushAnalyzer (AnalyzerFifo& fifo, const juce::AudioBuffer<float>& buf, int numCh);
    void updateEngineSettings();
    std::array<MultibandEngine::BandSettings, numBands> settings {};

    JUCE_DECLARE_NON_COPYABLE_WITH_LEAK_DETECTOR (MBC4Processor)
};
