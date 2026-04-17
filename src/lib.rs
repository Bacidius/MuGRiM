use nih_plug::prelude::*;
use nih_plug_webview::{WebViewEditor, HTMLSource};
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
    
    // NEW: The Mad Scientist's memory
    last_processed_step: usize,
    active_live_notes: Vec<u8>, 
}

impl Default for Mugrim {
    fn default() -> Self {
        Self {
            params: Arc::new(MugrimParams::default()),
            shared_memory: Arc::new(RwLock::new(SharedMemory::default())),
            last_processed_step: 9999, // A dummy value so step 0 triggers immediately
            active_live_notes: Vec::new(),
        }
    }
}

// --- 5. THE PARAMETERS ---
#[derive(Params)]
struct MugrimParams {

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
// --- THE AUDIO/MIDI THREAD ---
impl Plugin for Mugrim {
    const NAME: &'static str = "MuGRiM";
    const VENDOR: &'static str = "Aaron Wesley Arnold";
    const URL: &'static str = "https://github.com/Bacidius/MuGRiM";
    const EMAIL: &'static str = "DaddyOmega98049@gmail.com";
    const VERSION: &'static str = "0.1a";

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[]; 
    const MIDI_INPUT: MidiConfig = MidiConfig::None;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::MidiCCs; 
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;
    type SysExMessage = ();
    
    // NEW: We need state to track notes that are currently playing so we can turn them off
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> { self.params.clone() }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        let memory_bridge = self.shared_memory.clone();

        Some(Box::new(
            WebViewEditor::new(
                HTMLSource::String(include_str!("../ui/index.html")),
                (1000, 800)
            )
            .with_custom_protocol("mugrim".to_string(), move |request| {
                // The JS UI sends messages to a custom protocol (e.g., mugrim://action)
                // We extract the JSON body from the request path or body depending on how the UI sends it.
                // For this setup, we expect the UI to send a stringified JSON action.
                
                // Read the JSON string sent from the UI
                let message = String::from_utf8_lossy(request.body());

                if let Ok(action) = serde_json::from_str::<Action>(&message) {
                    let mut mem = memory_bridge.write().unwrap();
                    match action {
                        Action::SetLockZone { start, end, index: _ } => {
                            for i in start..=end { if i < 256 { mem.lock_map[i] = true; } }
                        },
                        Action::ClearLockZone { index: _ } => { mem.lock_map = [false; 256]; },
                        
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
                
                // The protocol handler expects a Result with a dynamic Cow byte slice
                Ok(nih_plug_webview::http::Response::builder()
                    .status(200)
                    .body(std::borrow::Cow::Owned(vec![]))
                    .unwrap())
            })
        ))
    }
    fn initialize(&mut self, _io: &AudioIOLayout, _cfg: &BufferConfig, _ctx: &mut impl InitContext<Self>) -> bool { true }
    fn reset(&mut self) {}
    
    // --- THE REAL ENGINE ---
    fn process(&mut self, _buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, context: &mut impl ProcessContext<Self>) -> ProcessStatus {
        let transport = context.transport();

        if transport.playing {
            if let Some(pos_beats) = transport.pos_beats() {
                // Calculate current 16th note
                let current_16th = (pos_beats * 4.0) as usize;
                let step_index = current_16th % 256;

                // We only want to trigger logic EXACTLY when the step changes, not every microsecond
                if step_index != self.last_processed_step {
                    self.last_processed_step = step_index;

                    // 1. Turn off any live random notes from the previous step
                    for pitch in self.active_live_notes.drain(..) {
                        context.send_event(nih_plug::midi::NoteEvent::NoteOff {
                            timing: 0, voice_id: None, channel: 0, note: pitch, velocity: 0.0,
                        });
                    }

                    if let Ok(mem) = self.shared_memory.try_read() {
                        // 2. Check the Lock Track!
                        if mem.lock_map[step_index] {
                            // --- THE ARCHITECT (Play Saved Notes) ---
                            for note in &mem.notes {
                                if note.start == step_index {
                                    context.send_event(nih_plug::midi::NoteEvent::NoteOn {
                                        timing: 0, voice_id: Some(note.id as i32), channel: 0, 
                                        note: note.pitch, velocity: note.velocity as f32 / 127.0, 
                                    });
                                }
                            }
                        } else {
                            // --- THE MAD SCIENTIST (Generative Mode) ---
                            
                            // Roll for a rest
                            let rest_prob = self.params.rest_probability.value();
                            if fastrand::f32() > rest_prob {
                                
                                // A simplified minor/Dorian interval map for the heavy riffs
                                // (We can expand this to all 31 modes later!)
                                let minor_intervals = [0, 2, 3, 5, 7, 8, 10]; 
                                
                                // Pick a random interval from the scale
                                let random_interval = minor_intervals[fastrand::usize(..minor_intervals.len())];
                                
                                // Roll for an octave jump (-1, 0, or +1 octave)
                                let octave_offsets = [-12i32, 0, 12];
                                let random_octave = octave_offsets[fastrand::usize(..octave_offsets.len())];
                                
                                // Calculate final pitch (Root + Interval + Octave)
                                let root = self.params.root_note.value(); // e.g., 4 for E
                                let base_octave = 36; // C2
                                let final_pitch = (base_octave + root + random_interval as i32 + random_octave) as u8;

                                // Keep it within bounds
                                if final_pitch >= 24 && final_pitch <= 84 {
                                    context.send_event(nih_plug::midi::NoteEvent::NoteOn {
                                        timing: 0, voice_id: None, channel: 0, 
                                        note: final_pitch, velocity: 0.8, // Hard hit for metal!
                                    });
                                    
                                    // Remember this note so we can kill it next step
                                    self.active_live_notes.push(final_pitch);
                                }
                            }
                        }
                        
                        // 3. Turn off SAVED Piano Roll notes that have reached their length
                        for note in &mem.notes {
                            if note.start + note.length == step_index {
                                context.send_event(nih_plug::midi::NoteEvent::NoteOff {
                                    timing: 0, voice_id: Some(note.id as i32), channel: 0, 
                                    note: note.pitch, velocity: 0.0,
                                });
                            }
                        }
                    }
                }
            }
        } else {
            // Panic Button: If DAW stops, kill all active random notes immediately
            for pitch in self.active_live_notes.drain(..) {
                context.send_event(nih_plug::midi::NoteEvent::NoteOff {
                    timing: 0, voice_id: None, channel: 0, note: pitch, velocity: 0.0,
                });
            }
            self.last_processed_step = 9999;
        }

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