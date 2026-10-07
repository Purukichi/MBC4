#pragma once

#include <juce_core/juce_core.h>

//==============================================================================
/** Lock-free single-producer / single-consumer sample FIFO (audio -> UI).
    The audio thread drops samples when the UI is not reading. */
class AnalyzerFifo
{
public:
    static constexpr int capacity = 16384;

    void push (const float* data, int num) noexcept
    {
        int s1, n1, s2, n2;
        fifo.prepareToWrite (num, s1, n1, s2, n2);
        if (n1 > 0) std::memcpy (buffer + s1, data,      (size_t) n1 * sizeof (float));
        if (n2 > 0) std::memcpy (buffer + s2, data + n1, (size_t) n2 * sizeof (float));
        fifo.finishedWrite (n1 + n2);
    }

    int pull (float* dest, int maxNum) noexcept
    {
        int s1, n1, s2, n2;
        fifo.prepareToRead (maxNum, s1, n1, s2, n2);
        if (n1 > 0) std::memcpy (dest,      buffer + s1, (size_t) n1 * sizeof (float));
        if (n2 > 0) std::memcpy (dest + n1, buffer + s2, (size_t) n2 * sizeof (float));
        fifo.finishedRead (n1 + n2);
        return n1 + n2;
    }

    int available() const noexcept { return fifo.getNumReady(); }

private:
    juce::AbstractFifo fifo { capacity };
    float buffer[capacity] {};
};
