use nih_plug::prelude::*;
use nih_plug_webview::{WebViewEditor, WebViewEditorState};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};

// --- 1. THE JSON BRIDGE (Web UI <-> Rust) ---
#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
enum Action {
    SetLockZone { start: usize, end: usize, index: usize }, 
    ClearLockZone { index: usize }, 
    ToggleLockMode { active: bool },
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum Event {
    UpdateNotes { notes: [Option<u8>; 256], current_step: usize },
}

// --- 2. THE SHARED MEMORY (The 16-Bar Buffer) ---
pub struct SharedMemory {
    pub note_buffer: [Option<u8>; 256], // 256 16th notes
    pub lock_map: [bool; 256],          // True if the user painted a blue lock zone here
    pub current_step: usize,
}

impl Default for SharedMemory {
    fn default() -> Self {
        Self {
            note_buffer: [None; 256],
            lock_map: [false; 256], // Everything starts unlocked
            current_step: 0,
        }
    }
}

// --- 3. THE PLUGIN CORE ---
struct Mugrim {
    params: Arc<MugrimParams>,
    shared_memory: Arc<RwLock<SharedMemory>>,
}

impl Default for Mugrim {
    fn default() -> Self {
        Self {
            params: Arc::new(MugrimParams::default()),
            // Initialize the bridge
            shared_memory: Arc::new(RwLock::new(SharedMemory::default())),
        }
    }
}

// --- 4. THE PARAMETERS ---
#[derive(Params)]
struct MugrimParams {
    // This tells the DAW to save the Web UI window size!
    #[persist = "editor-state"]
    pub editor_state: Arc<WebViewEditorState>,

    #[id = "rest_prob"]
    pub rest_probability: FloatParam,

    #[id = "repeat_prob"]
    pub repeat_probability: FloatParam,

    #[id = "phrase_prob"]
    pub phrase_repeat_prob: FloatParam,

    #[id = "phrase_length"]
    pub phrase_length: IntParam,

    #[id = "root_note"]
    pub root_note: IntParam,

    #[id = "min_pitch"]
    pub min_pitch: IntParam,

    #[id = "max_pitch"]
    pub max_pitch: IntParam,

    #[id = "max_jump"]
    pub max_jump: IntParam, 
}

impl Default for MugrimParams {
    fn default() -> Self {
        Self {
            // Set the default Web UI window size to 800x600
            editor_state: WebViewEditorState::from_size(800, 600),

            rest_probability: FloatParam::new("Rest Probability", 0.15, FloatRange::Linear { min: 0.0, max: 1.0 }),
            repeat_probability: FloatParam::new("Single Note Repeat", 0.3, FloatRange::Linear { min: 0.0, max: 1.0 }),
            phrase_repeat_prob: FloatParam::new("Phrase Repeat Chance", 0.25, FloatRange::Linear { min: 0.0, max: 1.0 }),
            phrase_length: IntParam::new("Phrase Length", 16, IntRange::Linear { min: 2, max: 64 }),
            root_note: IntParam::new("Root Note", 4, IntRange::Linear { min: 0, max: 11 }),
            min_pitch: IntParam::new("Lowest Note", 30, IntRange::Linear { min: 0, max: 127 }),
            max_pitch: IntParam::new("Highest Note", 52, IntRange::Linear { min: 0, max: 127 }),
            max_jump: IntParam::new("Max Jump", 12, IntRange::Linear { min: 1, max: 24 }),
        }
    }
}

// --- 5. THE AUDIO/MIDI THREAD ---
impl Plugin for Mugrim {
    const NAME: &'static str = "MuGRiM";
    const VENDOR: &'static str = "Aaron Wesley Arnold";
    const URL: &'static str = "https://...";
    const EMAIL: &'static str = "your@email.com";
    const VERSION: &'static str = "1.0.0";

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[]; 
    const MIDI_INPUT: MidiConfig = MidiConfig::None;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::MidiCCs; 

    const SAMPLE_ACCURATE_AUTOMATION: bool = true;
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    // --- THIS IS THE MAGIC WEBVIEW HOOK ---
    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        let memory_bridge = self.shared_memory.clone();

        WebViewEditor::new(
            self.params.editor_state.clone(),
            include_str!("../ui/index.html"),
            move |_context, message| {
                // When the HTML sends us a message, read the JSON!
                if let Ok(action) = serde_json::from_str::<Action>(&message) {
                    let mut mem = memory_bridge.write().unwrap();
                    match action {
                        Action::SetLockZone { start, end, index: _ } => {
                            for i in start..=end {
                                if i < 256 { mem.lock_map[i] = true; }
                            }
                        },
                        Action::ClearLockZone { index: _ } => {
                            // Clear all locks (simplified for now)
                            mem.lock_map = [false; 256]; 
                        },
                        _ => {}
                    }
                }
            },
        )
    }

    fn initialize(&mut self, _io: &AudioIOLayout, _cfg: &BufferConfig, _ctx: &mut impl InitContext<Self>) -> bool {
        true
    }

    fn reset(&mut self) {}

    fn process(&mut self, _buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &mut impl ProcessContext<Self>) -> ProcessStatus {
        // The audio/MIDI algorithm goes here!
        ProcessStatus::Normal
    }
}

impl ClapPlugin for Mugrim {
    const CLAP_ID: &'static str = "com.aaronarnold.mugrim";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Generative Hybrid Sequencer");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::NoteEffect, ClapFeature::Instrument];
}

impl Vst3Plugin for Mugrim {
    const VST3_CLASS_ID: [u8; 16] = *b"MuGRiMSeqExamp12"; 
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Instrument, Vst3SubCategory::Tools];
}

nih_export_clap!(Mugrim);
nih_export_vst3!(Mugrim);