#pragma once

#include <juce_dsp/juce_dsp.h>
#include <array>
#include <atomic>
#include <cmath>
#include <vector>

//==============================================================================
/**
    4-band compressor.

    Crossover: 4th-order Linkwitz-Riley tree with allpass compensation, so the
    bands sum back flat in magnitude.

    Per band (stereo linked, feed-forward):
      1. detection   L = 10 log10 (max (xL^2, xR^2))          [dB, instantaneous peak]
      2. gain curve  soft knee, ratio, clamped by range       -> reduction r >= 0 [dB]
      3. lookahead   r' = max of r over the next L samples, audio delayed by L
      4. ballistics  hold = max (r', release-smoothed hold);  gr = attack-smoothed hold
      5. output      gain = ((1 - mix) + mix * 10^(-gr/20)) * makeup

    Gain match: on request a band averages its effective gain change over a few
    seconds of signal; the UI writes the compensation into the band's Gain once.
*/
class MultibandEngine
{
public:
    static constexpr int numBands = 4;
    static constexpr double maxLookaheadMs = 10.0;

    struct BandSettings
    {
        float threshDb  = -18.0f;
        float ratio     = 2.0f;
        float attackMs  = 10.0f;
        float releaseMs = 120.0f;
        float kneeDb    = 6.0f;
        float makeupDb  = 0.0f;
        float rangeDb   = 60.0f;   // max gain reduction
        float mix       = 1.0f;    // band dry/wet (0..1)
        bool  bypass    = false;   // band passes through unprocessed
        bool  audible   = true;    // false when muted / not soloed
    };

    void prepare (double sampleRate, int maxBlockSize, int numChannels)
    {
        sr    = sampleRate;
        numCh = juce::jmax (1, numChannels);

        juce::dsp::ProcessSpec spec { sampleRate, (juce::uint32) juce::jmax (1, maxBlockSize), (juce::uint32) numCh };

        for (auto& f : split) { f.prepare (spec); f.setType (juce::dsp::LinkwitzRileyFilterType::lowpass); }
        for (auto& f : allpass) { f.prepare (spec); f.setType (juce::dsp::LinkwitzRileyFilterType::allpass); }

        for (auto& b : bandBuf)
            b.setSize (numCh, juce::jmax (1, maxBlockSize), false, false, true);

        for (auto& x : xSmooth)
            x.reset (sampleRate, 0.06);

        for (auto& g : bandGain)
            g.reset (sampleRate, 0.02);

        maxLookahead = (int) std::ceil (maxLookaheadMs * 0.001 * sr);
        for (auto& la : look)
        {
            la.delay.assign ((size_t) numCh * (size_t) (maxLookahead + 1), 0.0f);
            la.window.prepare (maxLookahead + 2);
        }

        reset();
    }

    void reset()
    {
        for (auto& f : split)   f.reset();
        for (auto& f : allpass) f.reset();

        for (auto& s : state) s = {};

        for (int i = 0; i < 3; ++i)
            xSmooth[i].setCurrentAndTargetValue (xTarget[i]);

        for (auto& g : bandGain)
            g.setCurrentAndTargetValue (1.0f);

        clearLookahead();
        applyCrossovers();
    }

    void setCrossovers (float f1, float f2, float f3)
    {
        // keep the splits ordered with at least ~1/4 octave between them
        const float nyq = (float) (sr * 0.45);
        f1 = juce::jlimit (20.0f, nyq / 1.4f, f1);
        f2 = juce::jlimit (f1 * 1.19f, nyq / 1.19f, f2);
        f3 = juce::jlimit (f2 * 1.19f, nyq, f3);

        xTarget = { f1, f2, f3 };
        for (int i = 0; i < 3; ++i)
            xSmooth[i].setTargetValue (xTarget[i]);
    }

    /** lookahead in samples (= added latency); clears the delay lines when it changes */
    void setLookahead (int samples)
    {
        samples = juce::jlimit (0, maxLookahead, samples);
        if (samples != lookahead)
        {
            lookahead = samples;
            clearLookahead();
        }
    }

    int getLookahead() const noexcept { return lookahead; }

    void process (juce::AudioBuffer<float>& buffer, const std::array<BandSettings, numBands>& settings, float mix)
    {
        const int n  = buffer.getNumSamples();
        const int ch = juce::jmin (buffer.getNumChannels(), numCh);
        if (n == 0 || ch == 0)
            return;

        // ---- per-block band coefficients --------------------------------
        std::array<Coeffs, numBands> c {};
        for (int b = 0; b < numBands; ++b)
        {
            const auto& s = settings[(size_t) b];
            auto& k = c[(size_t) b];
            k.thr      = s.threshDb;
            k.slope    = 1.0f / juce::jmax (1.0f, s.ratio) - 1.0f;
            k.knee     = juce::jmax (0.0f, s.kneeDb);
            k.halfKnee = k.knee * 0.5f;
            k.range    = -juce::jmax (0.0f, s.rangeDb);
            k.att      = std::exp (-1.0f / (juce::jmax (0.01f, s.attackMs)  * 0.001f * (float) sr));
            k.rel      = std::exp (-1.0f / (juce::jmax (1.0f,  s.releaseMs) * 0.001f * (float) sr));
            k.makeup   = s.makeupDb;
            k.mix      = juce::jlimit (0.0f, 1.0f, s.mix * juce::jlimit (0.0f, 1.0f, mix));
            k.active   = ! s.bypass;

            bandGain[(size_t) b].setTargetValue (s.audible ? 1.0f : 0.0f);
        }

        // ---- split into bands (sub-blocks so crossover moves stay smooth) -
        constexpr int chunk = 32;
        for (int start = 0; start < n; start += chunk)
        {
            const int len = juce::jmin (chunk, n - start);

            if (xSmooth[0].isSmoothing() || xSmooth[1].isSmoothing() || xSmooth[2].isSmoothing())
            {
                for (auto& x : xSmooth) x.skip (len);
                applyCrossovers();
            }

            for (int c0 = 0; c0 < ch; ++c0)
            {
                const float* in = buffer.getReadPointer (c0, start);
                float* b0 = bandBuf[0].getWritePointer (c0, start);
                float* b1 = bandBuf[1].getWritePointer (c0, start);
                float* b2 = bandBuf[2].getWritePointer (c0, start);
                float* b3 = bandBuf[3].getWritePointer (c0, start);

                for (int i = 0; i < len; ++i)
                {
                    float low, rest, mid, rest2, hiMid, high;
                    split[0].processSample (c0, in[i], low, rest);
                    split[1].processSample (c0, rest, mid, rest2);
                    split[2].processSample (c0, rest2, hiMid, high);

                    // phase compensation for the lower bands
                    low = allpass[0].processSample (c0, low);   // @ f2
                    low = allpass[1].processSample (c0, low);   // @ f3
                    mid = allpass[2].processSample (c0, mid);   // @ f3

                    b0[i] = low; b1[i] = mid; b2[i] = hiMid; b3[i] = high;
                }
            }
        }

        // ---- per band dynamics + sum -------------------------------------
        buffer.clear();
        const int delaySize = maxLookahead + 1;

        for (int b = 0; b < numBands; ++b)
        {
            auto& k  = c[(size_t) b];
            auto& st = state[(size_t) b];
            auto& la = look[(size_t) b];
            auto& gSm = bandGain[(size_t) b];
            auto& buf = bandBuf[(size_t) b];

            float minGr = 0.0f, maxLvl = -120.0f;
            const float makeupLin = juce::Decibels::decibelsToGain (k.makeup);

            // gain-match learning: average the effective gain change while the band has signal
            auto& ln = learn[(size_t) b];
            if (ln.resetRequest.exchange (false))
            {
                ln.sum = 0.0; ln.count = 0;
            }
            const bool learning = ln.active.load (std::memory_order_relaxed) && k.active;

            for (int i = 0; i < n; ++i)
            {
                // ---- 1. detection (linked peak, on the undelayed band signal) ----
                float pk2 = 0.0f;
                for (int c0 = 0; c0 < ch; ++c0)
                {
                    const float x = buf.getSample (c0, i);
                    pk2 = juce::jmax (pk2, x * x);
                }

                const float lvlDb = powerToDb (pk2);
                maxLvl = juce::jmax (maxLvl, lvlDb);

                // ---- 2. gain computer -> reduction (>= 0 dB) -----------------
                const float red = k.active ? -juce::jmax (k.range, computeGr (lvlDb, k.thr, k.slope, k.knee, k.halfKnee)) : 0.0f;

                // ---- 3. lookahead: worst reduction over the next L samples ----
                const float redLa = la.window.push (red, lookahead + 1);

                // ---- 4. ballistics (smooth decoupled peak detector) ----------
                float gain = 1.0f;
                if (k.active)
                {
                    st.hold = juce::jmax (redLa, k.rel * st.hold + (1.0f - k.rel) * redLa);
                    st.gr   = st.hold + k.att * (st.gr - st.hold);   // stored as a positive reduction
                    minGr = juce::jmin (minGr, -st.gr);

                    // ---- 5. output gain --------------------------------------
                    const float wet  = juce::Decibels::decibelsToGain (-st.gr);
                    const float comp = (1.0f - k.mix) + k.mix * wet;   // effective band gain change
                    gain = comp * makeupLin;

                    if (learning && lvlDb > learnFloorDb)
                    {
                        ln.sum += (double) juce::Decibels::gainToDecibels (comp, -60.0f);
                        ++ln.count;
                    }
                }
                else
                {
                    st.gr = st.hold = 0.0f;
                }

                gain *= gSm.getNextValue();

                // delay the audio by the lookahead and apply the gain to it
                const int w = la.writePos;
                const int r = w - lookahead < 0 ? w - lookahead + delaySize : w - lookahead;
                for (int c0 = 0; c0 < ch; ++c0)
                {
                    float* line = la.delay.data() + (size_t) c0 * (size_t) delaySize;
                    line[w] = buf.getSample (c0, i);
                    buffer.addSample (c0, i, line[r] * gain);
                }
                la.writePos = w + 1 == delaySize ? 0 : w + 1;
            }

            grMeter[(size_t) b].store (minGr, std::memory_order_relaxed);
            ln.publishedSum.store (ln.sum, std::memory_order_relaxed);
            ln.publishedCount.store (ln.count, std::memory_order_release);
            levelMeter[(size_t) b].store (maxLvl, std::memory_order_relaxed);
        }

        for (auto& f : split)   f.snapToZero();
        for (auto& f : allpass) f.snapToZero();
    }

    // static curve, also used by the UI
    static float computeGr (float x, float thr, float slope, float knee, float halfKnee) noexcept
    {
        const float over = x - thr;
        if (over <= -halfKnee)
            return 0.0f;
        if (over < halfKnee && knee > 0.0f)
        {
            const float t = over + halfKnee;
            return slope * t * t / (2.0f * knee);
        }
        return slope * over;
    }

    float getGainReductionDb (int band) const noexcept { return grMeter[(size_t) band].load (std::memory_order_relaxed); }
    float getBandLevelDb (int band) const noexcept     { return levelMeter[(size_t) band].load (std::memory_order_relaxed); }

    //==========================================================================
    // gain match learning (UI thread)
    void startLearning (int band) noexcept
    {
        auto& ln = learn[(size_t) band];
        ln.resetRequest.store (true);
        ln.publishedCount.store (0);
        ln.publishedSum.store (0.0);
        ln.active.store (true);
    }

    void stopLearning (int band) noexcept     { learn[(size_t) band].active.store (false); }
    bool isLearning (int band) const noexcept { return learn[(size_t) band].active.load(); }

    /** seconds of signal collected so far */
    double learnedSeconds (int band) const noexcept
    {
        return (double) learn[(size_t) band].publishedCount.load (std::memory_order_acquire) / sr;
    }

    /** average effective gain change in dB (<= 0) */
    float learnedAverageDb (int band) const noexcept
    {
        const auto& ln = learn[(size_t) band];
        const auto count = ln.publishedCount.load (std::memory_order_acquire);
        return count > 0 ? (float) (ln.publishedSum.load (std::memory_order_relaxed) / (double) count) : 0.0f;
    }

private:
    struct Coeffs
    {
        float thr, slope, knee, halfKnee, range, att, rel, makeup, mix;
        bool active;
    };

    static float powerToDb (float p) noexcept { return 10.0f * std::log10 (p + 1.0e-12f); }

    void applyCrossovers()
    {
        const float f1 = xSmooth[0].getCurrentValue();
        const float f2 = xSmooth[1].getCurrentValue();
        const float f3 = xSmooth[2].getCurrentValue();
        split[0].setCutoffFrequency (f1);
        split[1].setCutoffFrequency (f2);
        split[2].setCutoffFrequency (f3);
        allpass[0].setCutoffFrequency (f2);
        allpass[1].setCutoffFrequency (f3);
        allpass[2].setCutoffFrequency (f3);
    }

    void clearLookahead()
    {
        for (auto& la : look)
        {
            std::fill (la.delay.begin(), la.delay.end(), 0.0f);
            la.writePos = 0;
            la.window.clear();
        }
    }

    //==========================================================================
    /** running maximum over the last `window` values (monotonic queue, O(1) amortised) */
    struct SlidingMax
    {
        std::vector<float> val;
        std::vector<juce::int64> idx;
        int head = 0, size = 0, cap = 1;
        juce::int64 t = 0;

        void prepare (int capacity) { cap = juce::jmax (1, capacity); val.assign ((size_t) cap, 0.0f); idx.assign ((size_t) cap, 0); clear(); }
        void clear() { head = size = 0; t = 0; }

        float push (float v, int window) noexcept
        {
            // drop smaller values from the back, they can never be the maximum again
            while (size > 0 && val[(size_t) ((head + size - 1) % cap)] <= v)
                --size;
            const int back = (head + size) % cap;
            val[(size_t) back] = v; idx[(size_t) back] = t;
            ++size;

            // drop values that left the window from the front
            while (idx[(size_t) head] <= t - window)
            {
                head = (head + 1) % cap;
                --size;
            }
            ++t;
            return val[(size_t) head];
        }
    };

    struct Lookahead
    {
        std::vector<float> delay;   // numCh x (maxLookahead + 1)
        int writePos = 0;
        SlidingMax window;
    };

    struct BandState { float gr = 0.0f, hold = 0.0f; };

    struct Learn
    {
        std::atomic<bool> active { false }, resetRequest { false };
        double sum = 0.0;                     // audio thread only
        juce::int64 count = 0;
        std::atomic<double> publishedSum { 0.0 };
        std::atomic<juce::int64> publishedCount { 0 };
    };
    static constexpr float learnFloorDb = -60.0f;   // ignore silence while learning

    double sr = 44100.0;
    int numCh = 2;
    int maxLookahead = 0, lookahead = 0;

    std::array<juce::dsp::LinkwitzRileyFilter<float>, 3> split, allpass;
    std::array<juce::AudioBuffer<float>, numBands> bandBuf;
    std::array<BandState, numBands> state {};
    std::array<Lookahead, numBands> look;
    std::array<juce::SmoothedValue<float, juce::ValueSmoothingTypes::Multiplicative>, 3> xSmooth;
    std::array<float, 3> xTarget { 120.0f, 1000.0f, 6000.0f };
    std::array<juce::SmoothedValue<float>, numBands> bandGain;

    std::array<std::atomic<float>, numBands> grMeter {}, levelMeter {};
    std::array<Learn, numBands> learn;
};
