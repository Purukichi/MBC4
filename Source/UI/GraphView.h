#pragma once

#include <juce_gui_basics/juce_gui_basics.h>
#include <juce_dsp/juce_dsp.h>
#include "PluginProcessor.h"

//==============================================================================
/**
    Main graph: input/output spectrum, band regions, draggable crossovers and
    per-band threshold / gain-reduction display.

    Mouse:  drag crossover line  -> move split frequency (Alt+click: default)
            dbl-click freq tag   -> type a frequency
            drag inside a band   -> threshold   (Ctrl / Shift: fine)
            double-click         -> reset (crossover line / threshold)
*/
class GraphView : public juce::Component
{
public:
    enum class AnalyzerMode { both, post, pre, off };

    explicit GraphView (MBC4Processor&);

    /** Called from the editor timer (~30 Hz). */
    void updateFrame();

    void setSelectedBand (int b)            { if (selected != b) { selected = b; repaint(); } }
    void setAnalyzerMode (AnalyzerMode m)   { mode = m; repaint(); }
    AnalyzerMode getAnalyzerMode() const    { return mode; }

    std::function<void (int)> onBandSelected;

    /** plot rectangle and dB scale, so the side meters can share the same vertical grid */
    juce::Rectangle<float> getPlotArea() const noexcept { return plot; }
    static constexpr float minDb = -72.0f, maxDb = 6.0f;

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
    static constexpr float minHz = 20.0f,  maxHz = 20000.0f;

    struct Spectrum
    {
        std::array<float, fftSize> ring {};
        int writePos = 0;
        int newSamples = 0;
        std::vector<float> display;   // per plot point, dB
        juce::Path path, fill;
    };

    // geometry
    float freqToX (float f) const noexcept;
    float xToFreq (float x) const noexcept;
    float dbToY (float db) const noexcept;
    float crossoverHz (int i) const;
    float bandLeftX (int b) const;
    float bandRightX (int b) const;
    int   bandAt (float x) const;
    int   crossoverAt (juce::Point<float> p) const;
    juce::Rectangle<float> tagBounds (int i) const;

    void pullAndAnalyse (AnalyzerFifo&, Spectrum&);
    void buildPath (Spectrum&);
    void setParam (juce::RangedAudioParameter*, float value);
    void editCrossover (int i);
    void commitCrossoverEdit();

    MBC4Processor& proc;

    juce::RangedAudioParameter* xParam[3] {};
    juce::RangedAudioParameter* threshP[MBC4Processor::numBands] {};
    juce::RangedAudioParameter* ratioP[MBC4Processor::numBands] {};
    juce::RangedAudioParameter* soloP[MBC4Processor::numBands] {};
    juce::RangedAudioParameter* muteP[MBC4Processor::numBands] {};
    juce::RangedAudioParameter* bypassP[MBC4Processor::numBands] {};

    juce::dsp::FFT fft { fftOrder };
    juce::dsp::WindowingFunction<float> window { (size_t) fftSize, juce::dsp::WindowingFunction<float>::hann, false };
    std::vector<float> fftBuf, scratch;
    std::vector<float> pointHz;           // centre frequency of each plot point
    std::vector<float> pointX;
    Spectrum pre, post;
    std::vector<float> frame;             // one analysis frame before smoothing

    std::array<float, MBC4Processor::numBands> grDisp {}, lvlDisp {}, lvlHold {};
    std::array<int, MBC4Processor::numBands> lvlHoldTime {};

    juce::Rectangle<float> plot;

    AnalyzerMode mode = AnalyzerMode::both;
    int selected = 0;
    int hoverBand = -1, hoverX = -1;

    enum class Drag { none, crossover, threshold };
    Drag drag = Drag::none;
    int dragIndex = -1;
    float dragStartValue = 0.0f;
    float lastDragY = 0.0f;
    juce::RangedAudioParameter* dragParam = nullptr;

    juce::TextEditor freqEditor;
    int editingIndex = -1;

    JUCE_DECLARE_NON_COPYABLE_WITH_LEAK_DETECTOR (GraphView)
};
