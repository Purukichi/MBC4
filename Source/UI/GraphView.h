#pragma once

#include <juce_gui_basics/juce_gui_basics.h>
#include <juce_dsp/juce_dsp.h>
#include "PluginProcessor.h"
#include "Controls.h"

//==============================================================================
/**
    "02 SPLIT": the spectrum drawn with cells, crossovers and per-band thresholds.
    Output spectrum in grey steps, the part each band's compressor removes in orange,
    the input behind it in a pale grey.

    Mouse:  drag a crossover line  -> split frequency (Shift / Ctrl: fine)
            drag inside a band     -> its threshold, 1 dB steps (Shift / Ctrl: 0.1 dB)
            double-click           -> default (crossover / threshold)
*/
class GraphView : public juce::Component
{
public:
    enum class AnalyzerMode { both, post, pre, off };

    explicit GraphView (MBC4Processor&);

    /** Called from the editor timer (~30 Hz). */
    void updateFrame();

    void setAnalyzerMode (AnalyzerMode m);
    AnalyzerMode getAnalyzerMode() const noexcept { return mode; }
    std::function<void (AnalyzerMode)> onModeChanged;

    void paint (juce::Graphics&) override;
    void resized() override;

    void mouseMove (const juce::MouseEvent&) override;
    void mouseExit (const juce::MouseEvent&) override;
    void mouseDown (const juce::MouseEvent&) override;
    void mouseDrag (const juce::MouseEvent&) override;
    void mouseUp (const juce::MouseEvent&) override;
    void mouseDoubleClick (const juce::MouseEvent&) override;

private:
    static constexpr int fftOrder = 11;
    static constexpr int fftSize  = 1 << fftOrder;
    static constexpr float minHz = 20.0f, maxHz = 20000.0f;
    static constexpr int numCols = 225;   // analyser points (the drawn cell count follows the UI scale)

    struct Spectrum
    {
        std::array<float, fftSize> ring {};
        int writePos = 0;
        int newSamples = 0;
        std::array<float, numCols> display {};   // per cell column, dB
    };

    // geometry (local coordinates)
    juce::Rectangle<float> plot;
    float freqToX (float f) const noexcept;
    float xToFreq (float x) const noexcept;
    float dbToY (float db) const noexcept;
    float yToDb (float y) const noexcept;
    float crossoverHz (int i) const;
    float bandLeftX (int b) const;
    float bandRightX (int b) const;
    int   bandAt (float x) const;
    int   crossoverAt (juce::Point<float>) const;
    bool  nearThreshold (juce::Point<float>, int band) const;

    void pullAndAnalyse (AnalyzerFifo&, Spectrum&);
    void drainFifo (AnalyzerFifo&);
    void paintCells (juce::Graphics&);
    void setParam (juce::RangedAudioParameter*, float value);

    MBC4Processor& proc;
    SegmentPill modePill { { "IN+OUT", "OUT", "IN", "OFF" } };

    juce::RangedAudioParameter* xParam[3] {};
    juce::RangedAudioParameter* threshP[MBC4Processor::numBands] {};
    juce::RangedAudioParameter* bypassP[MBC4Processor::numBands] {};

    juce::dsp::FFT fft { fftOrder };
    juce::dsp::WindowingFunction<float> window { (size_t) fftSize, juce::dsp::WindowingFunction<float>::hann, false };
    std::vector<float> fftBuf, scratch;
    std::array<float, numCols> colHz {}, frame {};
    Spectrum pre, post;

    std::array<float, MBC4Processor::numBands> grDisp {};

    AnalyzerMode mode = AnalyzerMode::both;

    // hover / drag
    enum class Hot { none, plot, crossover, threshold };
    Hot hot = Hot::none;
    int hotIndex = -1;
    juce::Point<float> mouse;

    enum class Drag { none, crossover, threshold };
    Drag drag = Drag::none;
    int dragIndex = -1;
    float dragRaw = 0.0f, lastDragY = 0.0f;
    juce::RangedAudioParameter* dragParam = nullptr;

    JUCE_DECLARE_NON_COPYABLE_WITH_LEAK_DETECTOR (GraphView)
};
