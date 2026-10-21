// Offline checks: crossover flatness, compression, CPU cost, UI snapshot.
#include "PluginProcessor.h"
#include <juce_audio_utils/juce_audio_utils.h>
#include <iostream>

namespace
{
    void setParam (MBC4Processor& p, const juce::String& id, float value)
    {
        auto* param = p.apvts.getParameter (id);
        param->setValueNotifyingHost (param->convertTo0to1 (value));
    }

    int failures = 0;
    void check (bool ok, const juce::String& what)
    {
        std::cout << (ok ? "[ OK ] " : "[FAIL] ") << what << std::endl;
        if (! ok) ++failures;
    }

    float rmsDb (const juce::AudioBuffer<float>& b, int start, int num)
    {
        return juce::Decibels::gainToDecibels (b.getRMSLevel (0, start, num), -150.0f);
    }

    /** 1 s of a 400 Hz sine at -6 dBFS (inside band 2). Returns {rms change dB, output peak dB of the 2nd half}. */
    std::pair<float, float> runSine (MBC4Processor& p, double sr, int block)
    {
        constexpr int n = 48000;
        juce::AudioBuffer<float> buf (2, n);
        const float amp = juce::Decibels::decibelsToGain (-6.0f);
        for (int i = 0; i < n; ++i)
        {
            const float v = amp * std::sin (juce::MathConstants<float>::twoPi * 400.0f * (float) i / (float) sr);
            buf.setSample (0, i, v); buf.setSample (1, i, v);
        }
        const float inDb = rmsDb (buf, n / 2, n / 2);

        juce::MidiBuffer midi;
        for (int s = 0; s < n; s += block)
        {
            juce::AudioBuffer<float> view (buf.getArrayOfWritePointers(), 2, s, juce::jmin (block, n - s));
            p.processBlock (view, midi);
        }
        return { rmsDb (buf, n / 2, n / 2) - inDb, juce::Decibels::gainToDecibels (buf.getMagnitude (0, n / 2, n / 2), -150.0f) };
    }
}

int main (int argc, char** argv)
{
    juce::ScopedJuceInitialiser_GUI gui;
    constexpr double sr = 48000.0;
    constexpr int block = 256;

    // ---- 1. sum of bands is flat (ratio 1) ----------------------------------
    {
        MBC4Processor p;
        for (int b = 0; b < 4; ++b) setParam (p, MBC4Processor::bandParamId (b, "ratio"), 1.0f);
        p.prepareToPlay (sr, block);

        constexpr int n = 1 << 15;
        juce::AudioBuffer<float> buf (2, n);
        buf.clear();
        buf.setSample (0, 0, 1.0f);
        buf.setSample (1, 0, 1.0f);

        juce::MidiBuffer midi;
        for (int s = 0; s < n; s += block)
        {
            juce::AudioBuffer<float> view (buf.getArrayOfWritePointers(), 2, s, block);
            p.processBlock (view, midi);
        }

        juce::dsp::FFT fft (15);
        std::vector<float> data ((size_t) n * 2, 0.0f);
        std::copy (buf.getReadPointer (0), buf.getReadPointer (0) + n, data.begin());
        fft.performFrequencyOnlyForwardTransform (data.data());

        float lo = 100.0f, hi = -100.0f;
        for (int i = 1; i < n / 2; ++i)
        {
            const float f = (float) i * (float) sr / (float) n;
            if (f < 20.0f || f > 20000.0f) continue;
            const float db = juce::Decibels::gainToDecibels (data[(size_t) i]);
            lo = juce::jmin (lo, db); hi = juce::jmax (hi, db);
        }
        check (hi - lo < 0.05f && std::abs (hi) < 0.05f,
               "crossover sum flat 20Hz-20kHz (min " + juce::String (lo, 3) + " dB, max " + juce::String (hi, 3) + " dB)");
    }

    // ---- 2. compression on a 400 Hz sine (band 2) ------------------------------------
    {
        MBC4Processor p;
        setParam (p, MBC4Processor::bandParamId (1, "thresh"), -30.0f);
        setParam (p, MBC4Processor::bandParamId (1, "ratio"), 4.0f);
        setParam (p, MBC4Processor::bandParamId (1, "knee"), 0.0f);
        p.prepareToPlay (sr, block);

        constexpr int n = 48000;
        juce::AudioBuffer<float> buf (2, n);
        const float amp = juce::Decibels::decibelsToGain (-6.0f);
        for (int i = 0; i < n; ++i)
        {
            const float v = amp * std::sin (juce::MathConstants<float>::twoPi * 400.0f * (float) i / (float) sr);
            buf.setSample (0, i, v); buf.setSample (1, i, v);
        }
        const float inDb = rmsDb (buf, n / 2, n / 2);

        juce::MidiBuffer midi;
        for (int s = 0; s < n; s += block)
        {
            juce::AudioBuffer<float> view (buf.getArrayOfWritePointers(), 2, s, juce::jmin (block, n - s));
            p.processBlock (view, midi);
        }
        const float outDb = rmsDb (buf, n / 2, n / 2);
        // peak -6 dB, thr -30, 4:1 -> 24 dB over -> GR = 18 dB
        const float gr = outDb - inDb;
        // neighbouring bands leak a little uncompressed signal, so the sum reduces slightly less
        check (std::abs (p.engine.getGainReductionDb (1) + 17.9f) < 0.5f && gr < -15.0f, "400 Hz sine: expected ~-18 dB, got " + juce::String (gr, 2) + " dB (meter "
                                              + juce::String (p.engine.getGainReductionDb (1), 2) + ")");
    }

    // ---- 2b. band mix 0 % = no compression ---------------------------------
    {
        MBC4Processor p;
        setParam (p, MBC4Processor::bandParamId (1, "thresh"), -30.0f);
        setParam (p, MBC4Processor::bandParamId (1, "ratio"), 4.0f);
        setParam (p, MBC4Processor::bandParamId (1, "mix"), 0.0f);
        p.prepareToPlay (sr, block);
        const auto [change, peak] = runSine (p, sr, block);
        check (std::abs (change) < 0.1f, "band mix 0 %: level change " + juce::String (change, 2) + " dB");
    }

    // ---- 2c. gain match learns the average reduction and then stays fixed --
    {
        MBC4Processor p;
        setParam (p, MBC4Processor::bandParamId (1, "thresh"), -30.0f);
        setParam (p, MBC4Processor::bandParamId (1, "ratio"), 4.0f);
        setParam (p, MBC4Processor::bandParamId (1, "knee"), 0.0f);
        p.prepareToPlay (sr, block);
        p.engine.startLearning (1);
        runSine (p, sr, block);
        const float avg = p.engine.learnedAverageDb (1);
        p.engine.stopLearning (1);
        check (p.engine.learnedSeconds (1) > 0.9 && avg < -16.0f && avg > -19.0f,
               "gain match learn: average " + juce::String (avg, 2) + " dB over " + juce::String (p.engine.learnedSeconds (1), 2) + " s");

        setParam (p, MBC4Processor::bandParamId (1, "gain"), -avg);
        const auto [change, peak] = runSine (p, sr, block);
        check (std::abs (change) < 1.5f, "with match applied the sine level is restored: change " + juce::String (change, 2) + " dB");
    }

    // ---- 2d. defaults: thr 0 dB / 2:1 leave a -6 dB sine untouched -------------
    {
        MBC4Processor p;
        p.prepareToPlay (sr, block);
        const auto [change, peak] = runSine (p, sr, block);
        check (std::abs (change) < 0.1f, "defaults leave -6 dBFS untouched: change " + juce::String (change, 2) + " dB");
    }

    // ---- 2e. lookahead: latency reported, delayed impulse, still flat -----------
    {
        MBC4Processor p;
        for (int b = 0; b < 4; ++b) setParam (p, MBC4Processor::bandParamId (b, "ratio"), 1.0f);
        setParam (p, "lookahead", 5.0f);
        p.setRateAndBufferSizeDetails (sr, block);
        p.prepareToPlay (sr, block);

        constexpr int n = 1 << 15;
        juce::AudioBuffer<float> buf (2, n);
        buf.clear();
        buf.setSample (0, 0, 1.0f); buf.setSample (1, 0, 1.0f);
        juce::MidiBuffer midi;
        for (int s = 0; s < n; s += block)
        {
            juce::AudioBuffer<float> view (buf.getArrayOfWritePointers(), 2, s, block);
            p.processBlock (view, midi);
        }

        const int expected = juce::roundToInt (0.005 * sr);
        int firstNonZero = -1;
        for (int i = 0; i < n && firstNonZero < 0; ++i)
            if (std::abs (buf.getSample (0, i)) > 1.0e-6f) firstNonZero = i;

        juce::dsp::FFT fft (15);
        std::vector<float> data ((size_t) n * 2, 0.0f);
        std::copy (buf.getReadPointer (0), buf.getReadPointer (0) + n, data.begin());
        fft.performFrequencyOnlyForwardTransform (data.data());
        float lo = 100.0f, hi = -100.0f;
        for (int i = 1; i < n / 2; ++i)
        {
            const float f = (float) i * (float) sr / (float) n;
            if (f < 20.0f || f > 20000.0f) continue;
            const float db = juce::Decibels::gainToDecibels (data[(size_t) i]);
            lo = juce::jmin (lo, db); hi = juce::jmax (hi, db);
        }
        check (p.getLatencySamples() == expected && firstNonZero == expected && hi - lo < 0.05f,
               "lookahead 5 ms: latency " + juce::String (p.getLatencySamples()) + " / first sample at " + juce::String (firstNonZero)
               + " (expected " + juce::String (expected) + "), flat within " + juce::String (hi - lo, 3) + " dB");
    }

    // ---- 2f. lookahead catches the transient that a zero-latency comp lets through
    {
        auto overshoot = [&] (float lookMs)
        {
            MBC4Processor p;
            for (int b = 0; b < 4; ++b)   // every band compresses, so the onset cannot leak through an idle band
            {
                setParam (p, MBC4Processor::bandParamId (b, "thresh"), -30.0f);
                setParam (p, MBC4Processor::bandParamId (b, "ratio"), 20.0f);
                setParam (p, MBC4Processor::bandParamId (b, "knee"), 0.0f);
                setParam (p, MBC4Processor::bandParamId (b, "attack"), 1.0f);
            }
            setParam (p, "lookahead", lookMs);
            p.setRateAndBufferSizeDetails (sr, block);
            p.prepareToPlay (sr, block);

            // 0.5 s silence, then a -6 dBFS 400 Hz burst
            constexpr int n = 48000;
            juce::AudioBuffer<float> buf (2, n);
            buf.clear();
            const float amp = juce::Decibels::decibelsToGain (-6.0f);
            for (int i = n / 2; i < n; ++i)
            {
                const float fade = juce::jmin (1.0f, (float) (i - n / 2) / 48.0f);   // 1 ms fade-in, no click
                const float v = fade * amp * std::sin (juce::MathConstants<float>::twoPi * 400.0f * (float) (i - n / 2) / (float) sr);
                buf.setSample (0, i, v); buf.setSample (1, i, v);
            }
            juce::MidiBuffer midi;
            for (int s = 0; s < n; s += block)
            {
                juce::AudioBuffer<float> view (buf.getArrayOfWritePointers(), 2, s, juce::jmin (block, n - s));
                p.processBlock (view, midi);
            }
            const int lat = p.getLatencySamples();
            const float onset  = buf.getMagnitude (0, n / 2 + lat, 480);            // first 10 ms of the burst
            const float steady = buf.getMagnitude (0, n - 4800, 4800);              // last 100 ms
            return juce::Decibels::gainToDecibels (onset) - juce::Decibels::gainToDecibels (steady);
        };
        const float without = overshoot (0.0f), with = overshoot (5.0f);
        // what remains with lookahead is the crossover's own onset ringing summing bands at different gains
        check (with < 3.0f && with < without - 4.0f,
               "transient overshoot above steady state: " + juce::String (without, 1) + " dB without lookahead, "
               + juce::String (with, 1) + " dB with 5 ms");
    }

    // ---- 3. CPU cost ---------------------------------------------------------
    {
        MBC4Processor p;
        setParam (p, "lookahead", 5.0f);
        for (int b = 0; b < 4; ++b)
        {
            setParam (p, MBC4Processor::bandParamId (b, "thresh"), -30.0f);
        }
        p.prepareToPlay (sr, block);

        constexpr int seconds = 20;
        juce::AudioBuffer<float> buf (2, block);
        juce::Random rng (1);
        juce::MidiBuffer midi;

        const auto t0 = juce::Time::getHighResolutionTicks();
        for (int s = 0; s < (int) sr * seconds; s += block)
        {
            for (int c = 0; c < 2; ++c)
                for (int i = 0; i < block; ++i)
                    buf.setSample (c, i, rng.nextFloat() * 0.5f - 0.25f);
            p.processBlock (buf, midi);
        }
        const double elapsed = juce::Time::highResolutionTicksToSeconds (juce::Time::getHighResolutionTicks() - t0);
        const double pct = 100.0 * elapsed / seconds;
        std::cout << "       CPU: " << juce::String (elapsed * 1000.0 / seconds, 3) << " ms per second of stereo 48k audio ("
                  << juce::String (pct, 3) << " % of one core)" << std::endl;
        check (pct < 3.0, "CPU under 3% of one core");
    }

    // ---- 3b. UI scale survives closing / reopening the editor ------------------
    {
        MBC4Processor p;
        {
            std::unique_ptr<juce::AudioProcessorEditor> ed (p.createEditor());
            ed->setSize (1860, 1200);   // 150 %
        }
        std::unique_ptr<juce::AudioProcessorEditor> ed (p.createEditor());
        check (ed->getWidth() == 1860 && ed->getHeight() == 1200,
               "UI scale restored on reopen (" + juce::String (ed->getWidth()) + " x " + juce::String (ed->getHeight()) + ")");
    }

    // ---- 4. UI snapshot -------------------------------------------------------
    if (argc > 1)
    {
        MBC4Processor p;
        setParam (p, MBC4Processor::bandParamId (0, "thresh"), -14.0f);
        setParam (p, MBC4Processor::bandParamId (1, "thresh"), -38.0f);
        setParam (p, MBC4Processor::bandParamId (2, "thresh"), -36.0f);
        setParam (p, MBC4Processor::bandParamId (3, "thresh"), -31.0f);
        setParam (p, MBC4Processor::bandParamId (1, "ratio"), 4.0f);
        setParam (p, MBC4Processor::bandParamId (3, "gain"), 2.5f);
        setParam (p, MBC4Processor::bandParamId (2, "mix"), 60.0f);
        p.setRateAndBufferSizeDetails (sr, block);
        p.prepareToPlay (sr, block);

        std::unique_ptr<juce::AudioProcessorEditor> ed (p.createEditor());
        ed->setSize (1240, 800);   // base size: the 2nd argument sets the render scale

        // pinkish noise + a kick-like low sine so every band moves
        juce::AudioBuffer<float> buf (2, block);
        juce::Random rng (2);
        juce::MidiBuffer midi;
        float b0 = 0, b1 = 0, b2 = 0, phase = 0;
        for (int frame = 0; frame < 40; ++frame)
        {
            for (int k = 0; k < 6; ++k)
            {
                for (int i = 0; i < block; ++i)
                {
                    const float w = rng.nextFloat() * 2.0f - 1.0f;
                    b0 = 0.99765f * b0 + w * 0.0990460f;
                    b1 = 0.96300f * b1 + w * 0.2965164f;
                    b2 = 0.57000f * b2 + w * 1.0526913f;
                    phase += juce::MathConstants<float>::twoPi * 60.0f / (float) sr;
                    const float v = 0.12f * (b0 + b1 + b2 + w * 0.1848f) + 0.4f * std::sin (phase);
                    buf.setSample (0, i, v); buf.setSample (1, i, v);
                }
                p.processBlock (buf, midi);
            }
            juce::Thread::sleep (34);
            juce::Timer::callPendingTimersSynchronously();
        }

        // optional 2nd argument: render scale (e.g. 2 for a crisp hi-dpi image)
        const float renderScale = argc > 2 ? juce::jlimit (1.0f, 4.0f, juce::String (argv[2]).getFloatValue()) : 1.0f;
        auto img = ed->createComponentSnapshot (ed->getLocalBounds(), true, renderScale);
        const juce::File out { juce::String (argv[1]) };
        out.deleteFile();
        juce::FileOutputStream os (out);
        juce::PNGImageFormat().writeImageToStream (img, os);
        std::cout << "       snapshot -> " << out.getFullPathName() << std::endl;
    }

    std::cout << (failures == 0 ? "ALL PASSED" : "FAILURES: " + std::to_string (failures)) << std::endl;
    return failures;
}
