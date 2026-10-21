cmake_minimum_required(VERSION 3.22)

# macOS: one universal binary (Apple Silicon + Intel). Must be set before project().
if(APPLE)
    set(CMAKE_OSX_DEPLOYMENT_TARGET "10.13" CACHE STRING "Minimum macOS version")
    set(CMAKE_OSX_ARCHITECTURES "arm64;x86_64" CACHE STRING "Build architectures")
endif()

project(MBC4 VERSION 1.1.0 LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# JUCE: the copy shared with the other plugin projects, or fetched (e.g. on a build machine).
set(MBC4_JUCE_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../Plugin/JUCE" CACHE PATH "Path to JUCE")
if(EXISTS "${MBC4_JUCE_DIR}/CMakeLists.txt")
    add_subdirectory("${MBC4_JUCE_DIR}" "${CMAKE_BINARY_DIR}/JUCE")
else()
    include(FetchContent)
    FetchContent_Declare(JUCE
        GIT_REPOSITORY https://github.com/juce-framework/JUCE.git
        GIT_TAG        8.0.12
        GIT_SHALLOW    TRUE)
    FetchContent_MakeAvailable(JUCE)
endif()

set(MBC4_FORMATS VST3 Standalone)
if(APPLE)
    list(APPEND MBC4_FORMATS AU)
endif()

juce_add_plugin(MBC4
    VERSION                  1.1.0
    COMPANY_NAME             "ongakuushi"
    BUNDLE_ID                com.ongakuushi.mbc4
    PLUGIN_MANUFACTURER_CODE Ongk
    PLUGIN_CODE              Mbc4
    FORMATS                  ${MBC4_FORMATS}
    PRODUCT_NAME             "MBC4"
    DESCRIPTION              "4-band multiband compressor"
    IS_SYNTH                 FALSE
    NEEDS_MIDI_INPUT         FALSE
    NEEDS_MIDI_OUTPUT        FALSE
    IS_MIDI_EFFECT           FALSE
    EDITOR_WANTS_KEYBOARD_FOCUS FALSE
    VST3_CATEGORIES          Fx Dynamics
    AU_MAIN_TYPE             kAudioUnitType_Effect
    COPY_PLUGIN_AFTER_BUILD  FALSE)

target_sources(MBC4 PRIVATE
    Source/PluginProcessor.cpp
    Source/PluginEditor.cpp
    Source/UI/MBCLookAndFeel.cpp
    Source/UI/Controls.cpp
    Source/UI/GraphView.cpp
    Source/UI/BandStrip.cpp)

target_include_directories(MBC4 PRIVATE Source)

# UI typeface (Chivo Mono, SIL OFL 1.1 - Resources/Fonts/OFL.txt ships with the release)
juce_add_binary_data(MBC4Fonts SOURCES
    Resources/Fonts/ChivoMono-Light.ttf
    Resources/Fonts/ChivoMono-Regular.ttf)

target_compile_definitions(MBC4 PUBLIC
    JUCE_WEB_BROWSER=0
    JUCE_USE_CURL=0
    JUCE_VST3_CAN_REPLACE_VST2=0)

if(MSVC)
    target_compile_options(MBC4 PRIVATE /utf-8)
endif()

target_link_libraries(MBC4
    PRIVATE
        juce::juce_audio_utils
        juce::juce_dsp
        MBC4Fonts
    PUBLIC
        juce::juce_recommended_config_flags
        juce::juce_recommended_lto_flags
        juce::juce_recommended_warning_flags)

# Optional CLAP build: clone https://github.com/free-audio/clap-juce-extensions
# (with submodules) into libs/clap-juce-extensions to enable it.
if(EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/libs/clap-juce-extensions/CMakeLists.txt")
    add_subdirectory(libs/clap-juce-extensions EXCLUDE_FROM_ALL)
    clap_juce_extensions_plugin(TARGET MBC4
        CLAP_ID "com.ongakuushi.mbc4"
        CLAP_FEATURES audio-effect compressor multi-effects stereo)
endif()

# Offline test / snapshot tool (not part of the plugin). Reuses the plugin's
# compiled shared code, so it only borrows MBC4's include paths and defines.
add_executable(MBC4_Tests Tests/TestMain.cpp)
target_include_directories(MBC4_Tests PRIVATE $<TARGET_PROPERTY:MBC4,INCLUDE_DIRECTORIES>)
target_compile_definitions(MBC4_Tests PRIVATE $<TARGET_PROPERTY:MBC4,COMPILE_DEFINITIONS>)
target_compile_options(MBC4_Tests PRIVATE $<TARGET_PROPERTY:MBC4,COMPILE_OPTIONS>)
target_link_libraries(MBC4_Tests PRIVATE MBC4)
set_target_properties(MBC4_Tests PROPERTIES EXCLUDE_FROM_ALL TRUE)
