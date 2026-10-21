#pragma once

/** Layout grid (base size, 10 px module; cells are 3 x 3 px on a 4 px pitch).
    Every position in the UI is derived from these, so edges line up across
    the graph, the meters, the band strips and the side columns. */
namespace Grid
{
    constexpr int baseW = 1240, baseH = 800;

    // outer frame and columns
    constexpr int frameL = 20, frameT = 20, frameR = 1220, frameB = 760;
    constexpr int colL = 140, colR = 1100;           // input | main | output
    constexpr int logoB = 50;                         // logo cell bottom (input column)
    constexpr int rowB = 380;                         // graph | band strips
    constexpr int stripW = 240;

    // cells
    constexpr float pitch = 4.0f, cell = 3.0f;

    // graph plot (window coordinates): 225 x 65 cells at 100 %, +6 .. -72 dB
    constexpr int plotL = 150, plotR = 1050, plotT = 80, plotB = 340;
    constexpr int plotCols = 225, plotRows = 65;
    constexpr float maxDb = 6.0f, minDb = -72.0f;
    constexpr int readoutTop = 352;                   // meter peak readouts / frequency labels

    // vertical faders (window y): label row, 40 cells, value row
    constexpr int faderTop = 418, faderH = 202;
    constexpr int faderLabelY = 425, faderCellsY = 440, faderValueY = 612;
    constexpr int faderRows = 40;
    constexpr int faderW = 44, faderGrab = 28;        // painted width / clickable width

    // horizontal sliders (window y of the first row centre), from the strip's left edge
    constexpr int sliderY0 = 650, sliderPitch = 20, sliderCellsX = 46, sliderCells = 32;

    // band strip x offsets
    constexpr int thsX = 30, rngX = 75, gainX = 120, grX = 210;
}
