#include "PluginProcessor.h"
#include "PluginEditor.h"

namespace
{
    using APF = juce::AudioParameterFloat;
    using Attr = juce::AudioParameterFloatAttributes;

    juce::NormalisableRange<float> logRange (float lo, float hi)
    {
        return { lo, hi,
                 [] (float s, float e, float t) { return s * std::pow (e / s, t); },
                 [] (float s, float e, float v) { return std::log (v / s) / std::log (e / s); },
                 [] (float s, float e, float v) { return juce::jlimit (s, e, v); } };
    }

    juce::String freqText (float v, int)
    {
        return v >= 1000.0f ? juce::String (v / 1000.0f, v >= 10000.0f ? 1 : 2) + " kHz"
                            : juce::String (juce::roundToInt (v)) + " Hz";
    }

    juce::String msText (float v, int)
    {
        return v < 10.0f ? juce::String (v, 1) + " ms" : juce::String (juce::roundToInt (v)) + " ms";
    }

    juce::String dbText (float v, int)   { return juce::String (v, 1) + " dB"; }
    juce::String ratioText (float v, int) { return v >= 19.95f ? juce::String ("20:1") : juce::String (v, v < 10.0f ? 1 : 0) + ":1"; }
    juce::String pctText (float v, int)  { return juce::String (juce::roundToInt (v)) + " %"; }


    /** accepts "1200", "1.2k", "1.2 kHz", "850hz" */
    float freqFromText (const juce::String& t)
    {
        auto s = t.trim().toLowerCase().removeCharacters (" ").replace ("hz", "");
        const bool kilo = s.containsChar ('k');
        const float v = s.removeCharacters ("k").getFloatValue();
        return kilo ? v * 1000.0f : v;
    }
}

juce::AudioProcessorValueTreeState::ParameterLayout MBC4Processor::createLayout()
{
    juce::AudioProcessorValueTreeState::ParameterLayout layout;
    const auto pid = [] (const juce::String& id) { return juce::ParameterID { id, 1 }; };

    const float xDefaults[3] = { 120.0f, 1000.0f, 6000.0f };
    for (int i = 0; i < 3; ++i)
        layout.add (std::make_unique<APF> (pid (xoverParamId (i)), "Crossover " + juce::String (i + 1),
                                           logRange (20.0f, 20000.0f), xDefaults[i],
                                           Attr().withStringFromValueFunction (freqText)
                                                 .withValueFromStringFunction (freqFromText).withLabel ("Hz")));

    // slower in the low bands (long periods: fast gain changes would distort the waveform),
    // faster up top (short transients). Release stays well above the band's longest period.
    const float atk[numBands] = { 25.0f, 15.0f, 8.0f, 3.0f };
    const float rel[numBands] = { 250.0f, 180.0f, 120.0f, 80.0f };
    const char* names[numBands] = { "Low", "LowMid", "HiMid", "High" };

    for (int b = 0; b < numBands; ++b)
    {
        auto group = std::make_unique<juce::AudioProcessorParameterGroup> ("band" + juce::String (b + 1), names[b], "|");
        const juce::String pre = juce::String (names[b]) + " ";

        auto skewed = [] (float lo, float hi, float centre)
        {
            juce::NormalisableRange<float> r (lo, hi);
            r.setSkewForCentre (centre);
            return r;
        };

        group->addChild (std::make_unique<APF> (pid (bandParamId (b, "thresh")), pre + "Threshold",
                                                juce::NormalisableRange<float> (-60.0f, 0.0f, 0.1f), 0.0f,
                                                Attr().withStringFromValueFunction (dbText).withLabel ("dB")));
        group->addChild (std::make_unique<APF> (pid (bandParamId (b, "ratio")), pre + "Ratio",
                                                skewed (1.0f, 20.0f, 4.0f), 2.0f,
                                                Attr().withStringFromValueFunction (ratioText)));
        group->addChild (std::make_unique<APF> (pid (bandParamId (b, "attack")), pre + "Attack",
                                                skewed (0.1f, 300.0f, 15.0f), atk[b],
                                                Attr().withStringFromValueFunction (msText).withLabel ("ms")));
        group->addChild (std::make_unique<APF> (pid (bandParamId (b, "release")), pre + "Release",
                                                skewed (5.0f, 3000.0f, 200.0f), rel[b],
                                                Attr().withStringFromValueFunction (msText).withLabel ("ms")));
        group->addChild (std::make_unique<APF> (pid (bandParamId (b, "knee")), pre + "Knee",
                                                juce::NormalisableRange<float> (0.0f, 24.0f, 0.1f), 6.0f,
                                                Attr().withStringFromValueFunction (dbText).withLabel ("dB")));
        group->addChild (std::make_unique<APF> (pid (bandParamId (b, "range")), pre + "Range",
                                                skewed (0.0f, 60.0f, 18.0f), 60.0f,
                                                Attr().withStringFromValueFunction (dbText).withLabel ("dB")));
        group->addChild (std::make_unique<APF> (pid (bandParamId (b, "gain")), pre + "Gain",
                                                juce::NormalisableRange<float> (-24.0f, 24.0f, 0.1f), 0.0f,
                                                Attr().withStringFromValueFunction (dbText).withLabel ("dB")));
        group->addChild (std::make_unique<APF> (pid (bandParamId (b, "mix")), pre + "Mix",
                                                juce::NormalisableRange<float> (0.0f, 100.0f, 0.1f), 100.0f,
                                                Attr().withStringFromValueFunction (pctText).withLabel ("%")));
        group->addChild (std::make_unique<juce::AudioParameterBool> (pid (bandParamId (b, "solo")), pre + "Solo", false));
        group->addChild (std::make_unique<juce::AudioParameterBool> (pid (bandParamId (b, "mute")), pre + "Mute", false));
        group->addChild (std::make_unique<juce::AudioParameterBool> (pid (bandParamId (b, "bypass")), pre + "Bypass", false));

        layout.add (std::move (group));
    }

    layout.add (std::make_unique<APF> (pid ("in_gain"), "Input",
                                       juce::NormalisableRange<float> (-24.0f, 24.0f, 0.1f), 0.0f,
                                       Attr().withStringFromValueFunction (dbText).withLabel ("dB")));
    layout.add (std::make_unique<APF> (pid ("out_gain"), "Output",
                                       juce::NormalisableRange<float> (-24.0f, 24.0f, 0.1f), 0.0f,
                                       Attr().withStringFromValueFunction (dbText).withLabel ("dB")));
    layout.add (std::make_unique<APF> (pid ("lookahead"), "Lookahead",
                                       juce::NormalisableRange<float> (0.0f, (float) MultibandEngine::maxLookaheadMs, 0.1f), 0.0f,
                                       Attr().withStringFromValueFunction (msText).withLabel ("ms")));
    layout.add (std::make_unique<APF> (pid ("mix"), "Mix",
                                       juce::NormalisableRange<float> (0.0f, 100.0f, 0.1f), 100.0f,
                                       Attr().withStringFromValueFunction (pctText).withLabel ("%")));
    return layout;
}

//==============================================================================
MBC4Processor::MBC4Processor()
    : AudioProcessor (BusesProperties().withInput  ("Input",  juce::AudioChannelSet::stereo(), true)
                                       .withOutput ("Output", juce::AudioChannelSet::stereo(), true)),
      apvts (*this, nullptr, "MBC4", createLayout())
{
    for (int b = 0; b < numBands; ++b)
    {
        auto get = [&] (const char* n) { return apvts.getRawParameterValue (bandParamId (b, n)); };
        band[(size_t) b] = { get ("thresh"), get ("ratio"), get ("attack"), get ("release"), get ("knee"),
                             get ("gain"), get ("range"), get ("mix"),
                             get ("solo"), get ("mute"), get ("bypass") };
    }

    for (int i = 0; i < 3; ++i)
        xover[(size_t) i] = apvts.getRawParameterValue (xoverParamId (i));

    inGain  = apvts.getRawParameterValue ("in_gain");
    outGain = apvts.getRawParameterValue ("out_gain");
    mix       = apvts.getRawParameterValue ("mix");
    lookahead = apvts.getRawParameterValue ("lookahead");
}

bool MBC4Processor::isBusesLayoutSupported (const BusesLayout& layouts) const
{
    const auto out = layouts.getMainOutputChannelSet();
    if (out != juce::AudioChannelSet::mono() && out != juce::AudioChannelSet::stereo())
        return false;
    return layouts.getMainInputChannelSet() == out;
}

void MBC4Processor::prepareToPlay (double sampleRate, int samplesPerBlock)
{
    const int numCh = juce::jmax (getTotalNumInputChannels(), getTotalNumOutputChannels());
    engine.prepare (sampleRate, samplesPerBlock, numCh);
    engine.setCrossovers (xover[0]->load(), xover[1]->load(), xover[2]->load());
    engine.reset();

    inGainSm.reset (sampleRate, 0.03);
    outGainSm.reset (sampleRate, 0.03);
    inGainSm.setCurrentAndTargetValue (juce::Decibels::decibelsToGain (inGain->load()));
    outGainSm.setCurrentAndTargetValue (juce::Decibels::decibelsToGain (outGain->load()));

    monoSize = juce::jmax (1, samplesPerBlock);
    mono.allocate ((size_t) monoSize, true);

    // bypass path keeps the same latency as the processed path
    bypassDelay.setSize (juce::jmax (1, numCh), (int) std::ceil (MultibandEngine::maxLookaheadMs * 0.001 * sampleRate) + 1);
    bypassDelay.clear();
    bypassWrite = 0;

    updateLookahead();
}

int MBC4Processor::lookaheadSamples() const
{
    return juce::roundToInt (lookahead->load() * 0.001 * getSampleRate());
}

void MBC4Processor::updateLookahead()
{
    const int samples = lookaheadSamples();
    engine.setLookahead (samples);
    if (getLatencySamples() != engine.getLookahead())
        setLatencySamples (engine.getLookahead());   // host delay compensation
}

void MBC4Processor::processBlockBypassed (juce::AudioBuffer<float>& buffer, juce::MidiBuffer&)
{
    // delay the dry signal by the reported latency so bypassing stays time-aligned
    const int lat = getLatencySamples();
    const int size = bypassDelay.getNumSamples();
    const int ch = juce::jmin (buffer.getNumChannels(), bypassDelay.getNumChannels());
    if (lat <= 0 || size <= lat)
        return;

    for (int i = 0; i < buffer.getNumSamples(); ++i)
    {
        const int r = (bypassWrite - lat + size) % size;
        for (int c = 0; c < ch; ++c)
        {
            bypassDelay.setSample (c, bypassWrite, buffer.getSample (c, i));
            buffer.setSample (c, i, bypassDelay.getSample (c, r));
        }
        bypassWrite = (bypassWrite + 1) % size;
    }
}

void MBC4Processor::updateEngineSettings()
{
    bool anySolo = false;
    for (auto& p : band)
        anySolo = anySolo || p.solo->load() > 0.5f;

    for (int b = 0; b < numBands; ++b)
    {
        auto& p = band[(size_t) b];
        auto& s = settings[(size_t) b];
        s.threshDb  = p.thresh->load();
        s.ratio     = p.ratio->load();
        s.attackMs  = p.attack->load();
        s.releaseMs = p.release->load();
        s.kneeDb    = p.knee->load();
        s.makeupDb  = p.gain->load();
        s.rangeDb   = p.range->load();
        s.mix       = p.mix->load() * 0.01f;
        s.bypass    = p.bypass->load() > 0.5f;

        const bool soloed = p.solo->load() > 0.5f;
        s.audible = p.mute->load() < 0.5f && (! anySolo || soloed);
    }

    engine.setCrossovers (xover[0]->load(), xover[1]->load(), xover[2]->load());
}

void MBC4Processor::pushAnalyzer (AnalyzerFifo& fifo, const juce::AudioBuffer<float>& buf, int numCh)
{
    const int n = juce::jmin (buf.getNumSamples(), monoSize);
    if (numCh == 1)
    {
        fifo.push (buf.getReadPointer (0), n);
        return;
    }

    const float* l = buf.getReadPointer (0);
    const float* r = buf.getReadPointer (1);
    for (int i = 0; i < n; ++i)
        mono[i] = 0.5f * (l[i] + r[i]);
    fifo.push (mono, n);
}

void MBC4Processor::processBlock (juce::AudioBuffer<float>& buffer, juce::MidiBuffer&)
{
    juce::ScopedNoDenormals noDenormals;

    const int numIn  = getTotalNumInputChannels();
    const int numOut = getTotalNumOutputChannels();
    for (int ch = numIn; ch < numOut; ++ch)
        buffer.clear (ch, 0, buffer.getNumSamples());

    const int numCh = juce::jmin (numIn, buffer.getNumChannels(), 2);
    const int n = buffer.getNumSamples();
    if (numCh == 0 || n == 0)
        return;

    // host may exceed the announced block size
    if (n > monoSize)
    {
        monoSize = n;
        mono.allocate ((size_t) n, true);
        engine.prepare (getSampleRate(), n, numCh);
    }

    updateEngineSettings();
    updateLookahead();

    inGainSm.setTargetValue (juce::Decibels::decibelsToGain (inGain->load()));
    outGainSm.setTargetValue (juce::Decibels::decibelsToGain (outGain->load()));

    if (inGainSm.isSmoothing())
        inGainSm.applyGain (buffer, n);
    else
        buffer.applyGain (inGainSm.getCurrentValue());

    float inPk = 0.0f;
    for (int ch = 0; ch < numCh; ++ch)
        inPk = juce::jmax (inPk, buffer.getMagnitude (ch, 0, n));
    inPeakDb.store (juce::Decibels::gainToDecibels (inPk, -100.0f), std::memory_order_relaxed);

    pushAnalyzer (preFifo, buffer, numCh);

    engine.process (buffer, settings, mix->load() * 0.01f);

    if (outGainSm.isSmoothing())
        outGainSm.applyGain (buffer, n);
    else
        buffer.applyGain (outGainSm.getCurrentValue());

    float outPk = 0.0f;
    for (int ch = 0; ch < numCh; ++ch)
        outPk = juce::jmax (outPk, buffer.getMagnitude (ch, 0, n));
    outPeakDb.store (juce::Decibels::gainToDecibels (outPk, -100.0f), std::memory_order_relaxed);

    pushAnalyzer (postFifo, buffer, numCh);
}

//==============================================================================
juce::AudioProcessorEditor* MBC4Processor::createEditor()
{
    return new MBC4Editor (*this);
}

void MBC4Processor::getStateInformation (juce::MemoryBlock& destData)
{
    auto state = apvts.copyState();
    if (auto xml = state.createXml())
        copyXmlToBinary (*xml, destData);
}

void MBC4Processor::setStateInformation (const void* data, int sizeInBytes)
{
    if (auto xml = getXmlFromBinary (data, sizeInBytes))
        if (xml->hasTagName (apvts.state.getType()))
            apvts.replaceState (juce::ValueTree::fromXml (*xml));
}

juce::AudioProcessor* JUCE_CALLTYPE createPluginFilter()
{
    return new MBC4Processor();
}
