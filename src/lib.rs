use nih_plug::prelude::*;
use nih_plug_webview::{WebViewEditor, WebViewEditorState};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};

// --- 1. THE MIDI NOTE OBJECT ---
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct MidiNote {
    pub id: usize,
    pub pitch: u8,
    pub start: usize,
    pub length: usize,
    pub velocity: u8,
}

// --- 2. THE JSON BRIDGE ---
#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
enum Action {
    SetLockZone { start: usize, end: usize, index: usize }, 
    ClearLockZone { index: usize }, 
    ToggleSync { sync: bool },
    SetInternalBpm { bpm: f32 },
    SetRoot { root: i32 },
    SetMode { mode: i32 }, 
    
    // The Piano Roll Actions
    AddNote { id: usize, pitch: u8, start: usize, length: usize, velocity: u8 },
    UpdateNote { id: usize, pitch: u8, start: usize, length: usize, velocity: u8 },
    DeleteNote { id: usize },
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum Event {
    UpdateNotes { 
        notes: Vec<MidiNote>, 
        current_step: usize,
        host_tempo: f64 
    },
}

// --- 3. THE SHARED MEMORY ---
pub struct SharedMemory {
    pub notes: Vec<MidiNote>,           // Dynamic list of notes instead of a fixed array
    pub lock_map: [bool; 256],          // True if the user painted a red lock zone
    pub current_step: usize,
}

impl Default for SharedMemory {
    fn default() -> Self {
        Self {
            notes: Vec::new(),
            lock_map: [false; 256], 
            current_step: 0,
        }
    }
}

// --- 4. THE PLUGIN CORE ---
struct Mugrim {
    params: Arc<MugrimParams>,
    shared_memory: Arc<RwLock<SharedMemory>>,
}

impl Default for Mugrim {
    fn default() -> Self {
        Self {
            params: Arc::new(MugrimParams::default()),
            shared_memory: Arc::new(RwLock::new(SharedMemory::default())),
        }
    }
}

// --- 5. THE PARAMETERS ---
#[derive(Params)]
struct MugrimParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<WebViewEditorState>,

    // Generative Rules
    #[id = "rest_prob"] pub rest_probability: FloatParam,
    #[id = "repeat_prob"] pub repeat_probability: FloatParam,
    #[id = "phrase_prob"] pub phrase_repeat_prob: FloatParam,
    #[id = "phrase_length"] pub phrase_length: IntParam,
    #[id = "min_pitch"] pub min_pitch: IntParam,
    #[id = "max_pitch"] pub max_pitch: IntParam,
    #[id = "max_jump"] pub max_jump: IntParam, 
    #[id = "double_stops"] pub allow_double_stops: BoolParam,

    // Theory & Time
    #[id = "root_note"] pub root_note: IntParam,
    #[id = "scale_mode"] pub scale_mode: IntParam, 
    #[id = "ts_top"] pub time_sig_top: IntParam,
    #[id = "ts_bottom"] pub time_sig_bottom: IntParam,
    #[id = "sync_host"] pub sync_to_host: BoolParam,
    #[id = "internal_bpm"] pub internal_bpm: FloatParam,
}

impl Default for MugrimParams {
    fn default() -> Self {
        Self {
            editor_state: WebViewEditorState::from_size(800, 600),

            rest_probability: FloatParam::new("Rest Probability", 0.15, FloatRange::Linear { min: 0.0, max: 1.0 }),
            repeat_probability: FloatParam::new("Single Note Repeat", 0.3, FloatRange::Linear { min: 0.0, max: 1.0 }),
            phrase_repeat_prob: FloatParam::new("Phrase Repeat Chance", 0.25, FloatRange::Linear { min: 0.0, max: 1.0 }),
            phrase_length: IntParam::new("Phrase Length", 16, IntRange::Linear { min: 2, max: 64 }),
            min_pitch: IntParam::new("Lowest Note", 30, IntRange::Linear { min: 0, max: 127 }),
            max_pitch: IntParam::new("Highest Note", 52, IntRange::Linear { min: 0, max: 127 }),
            max_jump: IntParam::new("Max Jump", 12, IntRange::Linear { min: 1, max: 24 }),
            allow_double_stops: BoolParam::new("Allow Double Stops", false),

            root_note: IntParam::new("Root Note", 4, IntRange::Linear { min: 0, max: 11 }), // E
            scale_mode: IntParam::new("Scale Mode", 1, IntRange::Linear { min: 0, max: 30 }), // Dorian
            time_sig_top: IntParam::new("Time Sig Numerator", 4, IntRange::Linear { min: 2, max: 17 }),
            time_sig_bottom: IntParam::new("Time Sig Denominator", 4, IntRange::Linear { min: 2, max: 17 }),
            
            // THESE FIX YOUR ERROR:
            sync_to_host: BoolParam::new("Sync to DAW", true),
            internal_bpm: FloatParam::new("Internal BPM", 120.0, FloatRange::Linear { min: 20.0, max: 300.0 }),
        }
    }
}

// --- 6. THE AUDIO/MIDI THREAD ---
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

    fn params(&self) -> Arc<dyn Params> { self.params.clone() }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        let memory_bridge = self.shared_memory.clone();

        WebViewEditor::new(
            self.params.editor_state.clone(),
            include_str!("../ui/index.html"),
            move |_context, message| {
                if let Ok(action) = serde_json::from_str::<Action>(&message) {
                    let mut mem = memory_bridge.write().unwrap();
                    match action {
                        Action::SetLockZone { start, end, index: _ } => {
                            for i in start..=end { if i < 256 { mem.lock_map[i] = true; } }
                        },
                        Action::ClearLockZone { index: _ } => { mem.lock_map = [false; 256]; },
                        
                        // NEW: Piano Roll Modifications
                        Action::AddNote { id, pitch, start, length, velocity } => {
                            mem.notes.push(MidiNote { id, pitch, start, length, velocity });
                        },
                        Action::UpdateNote { id, pitch, start, length, velocity } => {
                            if let Some(note) = mem.notes.iter_mut().find(|n| n.id == id) {
                                note.pitch = pitch; note.start = start; 
                                note.length = length; note.velocity = velocity;
                            }
                        },
                        Action::DeleteNote { id } => {
                            mem.notes.retain(|n| n.id != id);
                        },
                        _ => {}
                    }
                }
            },
        )
    }

    fn initialize(&mut self, _io: &AudioIOLayout, _cfg: &BufferConfig, _ctx: &mut impl InitContext<Self>) -> bool { true }
    fn reset(&mut self) {}
    fn process(&mut self, _buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &mut impl ProcessContext<Self>) -> ProcessStatus {
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
    const VST3_CLASS_ID: [u8; 16] = *b"MuGRiMSeqExamp13"; 
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Instrument, Vst3SubCategory::Tools];
}

nih_export_clap!(Mugrim);
nih_export_vst3!(Mugrim);