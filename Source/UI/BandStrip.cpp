#include "BandStrip.h"
#include "MBCLookAndFeel.h"
#include "Grid.h"

juce::String tooltipText (const char* heading, const char* body)
{
    return juce::String (heading) + "\n" + juce::String::fromUTF8 (body);
}

namespace
{
    constexpr float meterRange = 24.0f;                 // GR meter: 0 .. 24 dB over 40 cells
    constexpr int readoutHoldFrames = 15;               // held GR value: refreshed every 0.5 s

    // strip-local geometry (the strip starts at window y = Grid::rowB)
    constexpr float headerY = 15.0f, rangeX = 70.0f;
    constexpr int   faderY = Grid::faderTop - Grid::rowB;
    constexpr float grCellsY = (float) (Grid::faderCellsY - Grid::rowB);
    constexpr float grLabelY = (float) (Grid::faderLabelY - Grid::rowB);
    constexpr float grValueY = (float) (Grid::faderValueY - Grid::rowB);

    juce::String shortFreq (float f)
    {
        return f >= 1000.0f ? juce::String (f / 1000.0f, f >= 10000.0f ? 1 : 2) + "k" : juce::String (juce::roundToInt (f));
    }

    const juce::String dash = juce::String::fromUTF8 ("\xe2\x80\x93");

    Steps dbSteps()   { return { 1.0f, 0.1f, {} }; }
    Steps ratioSteps(){ return { 1.0f, 0.1f, { 1.0f, 1.5f, 2.0f, 3.0f, 4.0f, 6.0f, 8.0f, 10.0f, 20.0f } }; }
    Steps atkSteps()  { return { 1.0f, 0.1f, { 0.1f, 0.3f, 1.0f, 2.0f, 3.0f, 5.0f, 8.0f, 10.0f, 15.0f, 20.0f, 25.0f, 30.0f, 50.0f, 80.0f, 100.0f, 150.0f, 200.0f, 300.0f } }; }
    Steps rlsSteps()  { return { 1.0f, 1.0f, { 5.0f, 10.0f, 20.0f, 30.0f, 50.0f, 80.0f, 120.0f, 180.0f, 250.0f, 350.0f, 500.0f, 750.0f, 1000.0f, 1500.0f, 2000.0f, 3000.0f } }; }
    Steps mixSteps()  { return { 5.0f, 1.0f, {} }; }
}

BandStrip::BandStrip (MBC4Processor& p, int b)
    : proc (p), band (b),
      thresh  (p.apvts, MBC4Processor::bandParamId (b, "thresh"),  "THS",  CellFader::Orientation::vertical,   dbSteps(),    Format::dB),
      range   (p.apvts, MBC4Processor::bandParamId (b, "range"),   "RNG",  CellFader::Orientation::vertical,   dbSteps(),    Format::dB),
      gain    (p.apvts, MBC4Processor::bandParamId (b, "gain"),    "GAIN", CellFader::Orientation::vertical,   dbSteps(),    Format::dB),
      ratio   (p.apvts, MBC4Processor::bandParamId (b, "ratio"),   "RAT",  CellFader::Orientation::horizontal, ratioSteps(), Format::ratio),
      knee    (p.apvts, MBC4Processor::bandParamId (b, "knee"),    "KNE",  CellFader::Orientation::horizontal, dbSteps(),    Format::dB),
      attack  (p.apvts, MBC4Processor::bandParamId (b, "attack"),  "ATK",  CellFader::Orientation::horizontal, atkSteps(),   Format::ms),
      release (p.apvts, MBC4Processor::bandParamId (b, "release"), "RLS",  CellFader::Orientation::horizontal, rlsSteps(),   Format::ms),
      mix     (p.apvts, MBC4Processor::bandParamId (b, "mix"),     "MIX",  CellFader::Orientation::horizontal, mixSteps(),   Format::percent)
{
    thresh.setTooltip  (tooltipText ("Threshold", "この値を超えた分を圧縮します。グラフの中を上下にドラッグしても変えられます。"));
    range.setTooltip   (tooltipText ("Range", "ゲインリダクションの上限です。"));
    gain.setTooltip    (tooltipText ("Gain", "バンドの出力ゲイン（メイクアップ）。"));
    ratio.setTooltip   (tooltipText ("Ratio", "圧縮の比率。"));
    knee.setTooltip    (tooltipText ("Knee", "スレッショルド付近を滑らかにする幅。"));
    attack.setTooltip  (tooltipText ("Attack", "圧縮がかかり始めるまでの速さ。"));
    release.setTooltip (tooltipText ("Release", "圧縮が戻るまでの速さ。"));
    mix.setTooltip     (tooltipText ("Mix", "バンド内のドライ／ウェットの割合。"));

    for (auto* f : { &thresh, &range, &gain, &ratio, &knee, &attack, &release, &mix })
    {
        f->isDimmed = [this] { return isBypassed(); };
        addAndMakeVisible (*f);
    }

    solo.setTooltip   (tooltipText ("Solo", "このバンドだけを聴きます。"));
    mute.setTooltip   (tooltipText ("Mute", "このバンドを消音します。"));
    bypass.setTooltip (tooltipText ("Bypass", "このバンドの圧縮を止めます。"));
    for (auto* btn : { &solo, &mute, &bypass })
        addAndMakeVisible (*btn);

    using BA = juce::AudioProcessorValueTreeState::ButtonAttachment;
    soloAtt   = std::make_unique<BA> (p.apvts, MBC4Processor::bandParamId (b, "solo"), solo);
    muteAtt   = std::make_unique<BA> (p.apvts, MBC4Processor::bandParamId (b, "mute"), mute);
    bypassAtt = std::make_unique<BA> (p.apvts, MBC4Processor::bandParamId (b, "bypass"), bypass);

    // gain match: learn the average gain reduction once and write the compensation into gain
    learn.setTooltip (tooltipText ("Gain match", "3 秒間の平均ゲインリダクションを測って、ゲインを 1 回だけ合わせます。もう一度押すと取り消します。"));
    learn.isDimmed = [this] { return isBypassed(); };
    learn.onClick = [this] { toggleLearn(); };
    addAndMakeVisible (learn);

    freqEditor.setFont (Fonts::of (Fonts::Role::value));
    freqEditor.setJustification (juce::Justification::centredLeft);
    freqEditor.setIndents (2, 0);
    freqEditor.setBorder ({});
    freqEditor.setInputRestrictions (10, "0123456789.kKhHzZ ");
    freqEditor.onReturnKey = [this] { commitCrossover(); };
    freqEditor.onEscapeKey = [this] { editingCrossover = -1; freqEditor.setVisible (false); repaint(); };
    freqEditor.onFocusLost = [this] { editingCrossover = -1; freqEditor.setVisible (false); repaint(); };
    addChildComponent (freqEditor);

    lastBypass = isBypassed();
}

//==============================================================================
bool BandStrip::isBypassed() const
{
    return proc.apvts.getRawParameterValue (MBC4Processor::bandParamId (band, "bypass"))->load() > 0.5f;
}

float BandStrip::edgeHz (int crossover) const
{
    if (crossover < 0) return 20.0f;
    if (crossover > 2) return 20000.0f;
    return proc.apvts.getRawParameterValue (MBC4Processor::xoverParamId (crossover))->load();
}

juce::String BandStrip::rangeLo() const { return shortFreq (edgeHz (band - 1)); }
juce::String BandStrip::rangeHi() const { return shortFreq (edgeHz (band)); }

juce::Rectangle<float> BandStrip::loTextArea() const
{
    return { rangeX - 2.0f, headerY - 7.0f, Draw::width (rangeLo(), Fonts::Role::value) + 4.0f, 14.0f };
}

juce::Rectangle<float> BandStrip::hiTextArea() const
{
    const float x = rangeX + Draw::width (rangeLo() + dash, Fonts::Role::value);
    return { x - 2.0f, headerY - 7.0f, Draw::width (rangeHi(), Fonts::Role::value) + 4.0f, 14.0f };
}

//==============================================================================
void BandStrip::toggleLearn()
{
    if (isBypassed())
        return;

    if (proc.engine.isLearning (band))
        proc.engine.stopLearning (band);
    else
        proc.engine.startLearning (band);

    updateLearn();
}

void BandStrip::updateLearn()
{
    if (! proc.engine.isLearning (band))
    {
        learn.setToggleState (false, juce::dontSendNotification);
        return;
    }

    if (proc.engine.learnedSeconds (band) >= learnSeconds)
    {
        // done: store the compensation once; it does not move afterwards
        proc.engine.stopLearning (band);
        const float avg = proc.engine.learnedAverageDb (band);
        if (auto* p = proc.apvts.getParameter (MBC4Processor::bandParamId (band, "gain")))
        {
            p->beginChangeGesture();
            p->setValueNotifyingHost (p->convertTo0to1 (juce::jlimit (0.0f, 24.0f, -avg)));
            p->endChangeGesture();
        }
        learn.setToggleState (false, juce::dontSendNotification);
        return;
    }

    learn.setToggleState (true, juce::dontSendNotification);
}

void BandStrip::editCrossover (int index, juce::Rectangle<float> area)
{
    editingCrossover = index;
    freqEditor.setBounds (area.withWidth (juce::jmax (area.getWidth(), 44.0f)).toNearestInt());
    freqEditor.setText (shortFreq (edgeHz (index)), false);
    freqEditor.setVisible (true);
    freqEditor.grabKeyboardFocus();
    freqEditor.selectAll();
    repaint();
}

void BandStrip::commitCrossover()
{
    const int index = editingCrossover;
    editingCrossover = -1;
    freqEditor.setVisible (false);

    const auto text = freqEditor.getText();
    if (index < 0 || index > 2 || text.trim().isEmpty())
        return;

    auto* p = proc.apvts.getParameter (MBC4Processor::xoverParamId (index));
    const float lo = index > 0 ? edgeHz (index - 1) * 1.19f : 20.0f;
    const float hi = index < 2 ? edgeHz (index + 1) / 1.19f : 20000.0f;
    const float f  = juce::jlimit (lo, hi, p->convertFrom0to1 (p->getValueForText (text)));
    p->beginChangeGesture();
    p->setValueNotifyingHost (p->convertTo0to1 (f));
    p->endChangeGesture();

    lastLo = lastHi = -1.0f;   // refresh the header
    repaint();
}

void BandStrip::mouseDoubleClick (const juce::MouseEvent& e)
{
    // band edges in the header: the low edge is crossover band-1, the high edge crossover band
    if (band > 0 && loTextArea().contains (e.position))
        editCrossover (band - 1, loTextArea());
    else if (band < 3 && hiTextArea().contains (e.position))
        editCrossover (band, hiTextArea());
}

void BandStrip::updateMeter()
{
    const float gr = isBypassed() ? 0.0f : juce::jmin (meterRange, -proc.engine.getGainReductionDb (band));   // positive dB
    const float prevShown = grShown, prevReadout = grReadout;
    grShown = gr > grShown ? gr : grShown + (gr - grShown) * 0.25f;

    // held value: the peak of each half second, so it can be read
    grWindowPeak = juce::jmax (grWindowPeak, gr);
    if (++readoutFrames >= readoutHoldFrames)
    {
        grReadout = grWindowPeak;
        grWindowPeak = 0.0f;
        readoutFrames = 0;
    }

    if (proc.engine.isLearning (band) || learn.getToggleState())
        updateLearn();

    const float lo = edgeHz (band - 1), hi = edgeHz (band);
    const bool byp = isBypassed();

    if (byp != lastBypass || lo != lastLo || hi != lastHi)
    {
        lastBypass = byp; lastLo = lo; lastHi = hi;
        repaint();
        return;
    }

    if (std::abs (prevShown - grShown) > 0.01f || prevReadout != grReadout)
        repaint (juce::Rectangle<int> (Grid::grX - 24, (int) grLabelY - 8, 48, (int) (grValueY - grLabelY) + 16));
}

//==============================================================================
void BandStrip::resized()
{
    using namespace Grid;
    auto vfader = [] (CellFader& f, int cx) { f.setBounds (cx - faderW / 2, faderY, faderW, faderH); };
    vfader (thresh, thsX);
    vfader (range,  rngX);
    vfader (gain,   gainX);

    learn.setBounds (144, faderCellsY - rowB + 4 * faderRows - 16, 42, 16);

    int j = 0;
    for (auto* s : { &ratio, &knee, &attack, &release, &mix })
        s->setBounds (6, sliderY0 - rowB + sliderPitch * j++ - 7, 228, 14);

    int x = 166;
    for (auto* btn : { &solo, &mute, &bypass })
    {
        btn->setBounds (x, 7, 18, 16);
        x += 20;
    }
}

void BandStrip::paint (juce::Graphics& g)
{
    const Draw::Px px (g, *this);
    using R = Fonts::Role;
    const bool dim = isBypassed();

    Draw::sectionHeader (g, "0" + juce::String (band + 3), "BAND " + juce::String::charToString ((juce::juce_wchar) ('A' + band)),
                         10.0f, headerY, dim ? Palette::dText : Palette::ink);
    if (! freqEditor.isVisible())
        Draw::text (g, rangeLo() + dash + rangeHi() + " Hz", rangeX, headerY, R::value, dim ? Palette::dText : Palette::ink2);

    // gain reduction: orange cells hanging from the top
    const float cx = (float) Grid::grX;
    const int lit = juce::roundToInt (juce::jlimit (0.0f, 1.0f, grShown / meterRange) * (float) Grid::faderRows);
    juce::RectangleList<float> on, off;
    for (int j = 0; j < Grid::faderRows; ++j)
    {
        auto& list = j < lit ? on : off;
        const float y = grCellsY + Grid::pitch * (float) j;
        list.addWithoutMerging (px.cell (cx - 3.5f, y));
        list.addWithoutMerging (px.cell (cx + 0.5f, y));
    }
    g.setColour (Palette::off);  g.fillRectList (off);
    g.setColour (dim ? Palette::dLit : Palette::accent); g.fillRectList (on);

    Draw::text (g, "GR", cx, grLabelY, R::label, dim ? Palette::dText : Palette::ink, juce::Justification::centred);
    const bool reducing = ! dim && grReadout >= 0.05f;
    Draw::text (g, reducing ? "-" + juce::String (grReadout, 1) + " dB" : juce::String ("0.0 dB"), cx, grValueY, R::value,
                dim ? Palette::dText : reducing ? Palette::accent : Palette::ink3, juce::Justification::centred);
}
