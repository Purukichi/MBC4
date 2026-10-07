#pragma once

/** Layout grid (base size, 8 px unit). Every position in the UI is derived from these,
    so edges line up across the graph, the band strips and the side panel. */
namespace Grid
{
    constexpr int u       = 8;
    constexpr int baseW   = 1240, baseH = 800;
    constexpr int margin  = 3 * u;     // window edge
    constexpr int gutter  = 2 * u;     // between blocks

    constexpr int topBarH = 3 * u;
    constexpr int segW    = 7 * u;     // analyzer segment width
    constexpr int sideW   = 15 * u;    // side panel
    constexpr int graphH  = 39 * u;

    // graph insets around the plot: tag row above, frequency labels below, dB labels right
    constexpr int plotTop = 4 * u, plotBottom = 3 * u, plotRight = 5 * u;

    // band strip
    constexpr int pad      = 2 * u;    // inner padding of strips / side panel
    constexpr int headerH  = 3 * u;
    constexpr int knobTop  = 7 * u;    // first knob row, from the strip top
    constexpr int cellW    = 7 * u;
    constexpr int rowH     = 13 * u;
    constexpr int knobD    = 5 * u;
    constexpr int labelGap = u;
    constexpr int labelH   = 2 * u;
    constexpr int knobBlock = knobD + labelGap + labelH;      // disc + label
    constexpr int knobInset = (rowH - knobBlock) / 2;         // block offset inside its row

    constexpr int meterW   = 3;
    constexpr int buttonD  = 3 * u;
    constexpr int smallButtonD = 20;
}
