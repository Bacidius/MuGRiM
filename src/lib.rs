use std::sync::{ Arc, Mutex };
use std::sync::atomic::AtomicU32; // <-- This gets its own clean line
use nih_plug::prelude::{
    nih_export_clap,
    nih_export_vst3,
    AudioIOLayout,
    BufferConfig,
    Buffer,
    AuxiliaryBuffers,
    Editor,
    AsyncExecutor, // <-- Underscore removed here
    FloatParam,
    IntParam,
    BoolParam,
    FloatRange,
    IntRange,
    InitContext,
    Params,
    MidiConfig,
    Plugin,
    ProcessContext,
    ProcessStatus,
    ClapPlugin,
    ClapFeature,
    Vst3Plugin,
    Vst3SubCategory,
};
use nih_plug_webview::{ WebViewEditor, HTMLSource };
use serde::{ Deserialize, Serialize };

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
    SetParameter {
        param: String,
        value: f32,
    }, // <--- The Master Slider Link
    AddNote {
        id: usize,
        pitch: u8,
        start: usize,
        length: usize,
        velocity: u8,
    },
    UpdateNote {
        id: usize,
        pitch: u8,
        start: usize,
        length: usize,
        velocity: u8,
    },
    DeleteNote {
        id: usize,
    },
    SetLockZone {
        start: usize,
        end: usize,
        _index: usize,
    },
    ClearLockZone {
        _index: usize,
    },
    GetPlayhead,
    SetRoot {
        root: i32,
    },
    SetMode {
        mode: i32,
    },
    SetInternalBpm {
        bpm: f32,
    },
    ToggleSync {
        sync: bool,
    },
}

#[derive(Serialize)]
#[serde(tag = "type")]
#[allow(dead_code)]
enum Event {
    UpdateNotes {
        notes: Vec<MidiNote>,
        current_step: usize,
        host_tempo: f64,
    },
    UpdatePlayhead {
        step: u32,
    },
}

// --- 3. THE UNIFIED MEMORY ---
pub struct MugrimMemory {
    notes: Vec<MidiNote>,
    lock_map: [bool; 256],
}

// Manual Default to fix the [bool; 256] array size limit error
impl Default for MugrimMemory {
    fn default() -> Self {
        Self {
            notes: Vec::new(),
            lock_map: [false; 256],
        }
    }
}

// --- 4. THE PARAMETERS ---
#[derive(Params)]
pub struct MugrimParams {
    pub rest_probability: FloatParam,
    pub repeat_probability: FloatParam,
    pub phrase_repeat_prob: FloatParam,
    pub phrase_length: IntParam,
    pub min_pitch: IntParam,
    pub max_pitch: IntParam,
    pub max_jump: IntParam,
    pub allow_double_stops: BoolParam,
    pub root_note: IntParam,
    pub scale_mode: IntParam,
    pub time_sig_top: IntParam,
    pub time_sig_bottom: IntParam,
    pub sync_to_host: BoolParam,
    pub internal_bpm: FloatParam,
    pub min_note_length: IntParam,
    pub max_note_length: IntParam,
    pub allow_note_overlap: BoolParam,
    pub mem: Arc<Mutex<MugrimMemory>>,
    pub active_step: Arc<AtomicU32>,
    pub volatility: FloatParam,
}

impl Default for MugrimParams {
    fn default() -> Self {
        Self {
            rest_probability: FloatParam::new("Rest Probability", 0.15, FloatRange::Linear {
                min: 0.0,
                max: 1.0,
            }),
            repeat_probability: FloatParam::new("Single Note Repeat", 0.3, FloatRange::Linear {
                min: 0.0,
                max: 1.0,
            }),
            phrase_repeat_prob: FloatParam::new("Phrase Repeat Chance", 0.25, FloatRange::Linear {
                min: 0.0,
                max: 1.0,
            }),
            phrase_length: IntParam::new("Phrase Length", 16, IntRange::Linear { min: 2, max: 64 }),
            min_pitch: IntParam::new("Lowest Note", 30, IntRange::Linear { min: 0, max: 127 }),
            max_pitch: IntParam::new("Highest Note", 52, IntRange::Linear { min: 0, max: 127 }),
            max_jump: IntParam::new("Max Jump", 12, IntRange::Linear { min: 1, max: 24 }),
            allow_double_stops: BoolParam::new("Allow Double Stops", false),
            root_note: IntParam::new("Root Note", 4, IntRange::Linear { min: 0, max: 11 }),
            scale_mode: IntParam::new("Scale Mode", 1, IntRange::Linear { min: 0, max: 30 }),
            time_sig_top: IntParam::new("Time Sig Numerator", 4, IntRange::Linear {
                min: 2,
                max: 17,
            }),
            time_sig_bottom: IntParam::new("Time Sig Denominator", 4, IntRange::Linear {
                min: 2,
                max: 17,
            }),
            sync_to_host: BoolParam::new("Sync to DAW", true),
            internal_bpm: FloatParam::new("Internal BPM", 120.0, FloatRange::Linear {
                min: 20.0,
                max: 300.0,
            }),
            min_note_length: IntParam::new("Min Note Length", 1, IntRange::Linear {
                min: 1,
                max: 16,
            }),
            max_note_length: IntParam::new("Max Note Length", 4, IntRange::Linear {
                min: 1,
                max: 16,
            }),
            allow_note_overlap: BoolParam::new("Allow Overlap", false),
            mem: Arc::new(Mutex::new(MugrimMemory::default())),
            active_step: Arc::new(AtomicU32::new(9999)),
            volatility: FloatParam::new("Volatility", 12.0, FloatRange::Linear {
                min: 1.0,
                max: 13.0,
            }),
        }
    }
}

// --- 5. THE PLUGIN CORE ---
struct Mugrim {
    params: Arc<MugrimParams>,
    last_processed_step: usize,
    active_live_notes: Vec<u8>,
    last_pitch: u8,
    cumulative_jump: i32,
}

impl Default for Mugrim {
    fn default() -> Self {
        Self {
            params: Arc::new(MugrimParams::default()),
            last_processed_step: 9999,
            active_live_notes: Vec::new(),
            last_pitch: 41, // Default to a central note
            cumulative_jump: 0,
        }
    }
}

// --- 6. THE AUDIO/MIDI THREAD ---
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
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        let html = include_str!("../ui/index.html");
        let css = include_str!("../ui/style.css");
        let js = include_str!("../ui/script.js");

        let final_html = html
            .replace(
                "<link rel=\"stylesheet\" href=\"style.css\">",
                &format!("<style>{}</style>", css)
            )
            .replace("<script src=\"script.js\"></script>", &format!("<script>{}</script>", js));

        let params = self.params.clone();

        Some(
            Box::new(
                WebViewEditor::new(
                    HTMLSource::String(Box::leak(final_html.into_boxed_str())),
                    (1000, 800)
                )
                    .with_background_color((12, 35, 64, 255))
                    .with_event_loop(move |window_handler, setter, _window| {
                        while let Ok(json_value) = window_handler.next_event() {
                            if let Ok(action) = serde_json::from_value::<Action>(json_value) {
                                let mut mem = params.mem.lock().unwrap();

                                match action {
                                    Action::SetParameter { param, value } => {
                                        match param.as_str() {
                                            "rest_prob" =>
                                                setter.set_parameter(
                                                    &params.rest_probability,
                                                    value
                                                ),
                                            "volatility" =>
                                                setter.set_parameter(&params.volatility, value),
                                            "min_pitch" =>
                                                setter.set_parameter(
                                                    &params.min_pitch,
                                                    value as i32
                                                ),
                                            "max_pitch" =>
                                                setter.set_parameter(
                                                    &params.max_pitch,
                                                    value as i32
                                                ),
                                            "min_note_length" =>
                                                setter.set_parameter(
                                                    &params.min_note_length,
                                                    value as i32
                                                ),
                                            "max_note_length" =>
                                                setter.set_parameter(
                                                    &params.max_note_length,
                                                    value as i32
                                                ),
                                            _ => (),
                                        }
                                    }
                                    Action::AddNote { id, pitch, start, length, velocity } => {
                                        mem.notes.push(MidiNote {
                                            id,
                                            pitch,
                                            start,
                                            length,
                                            velocity,
                                        });
                                    }
                                    Action::UpdateNote { id, pitch, start, length, velocity } => {
                                        // Direct indexing bypasses the MutexGuard borrow checker confusion
                                        for i in 0..mem.notes.len() {
                                            if mem.notes[i].id == id {
                                                mem.notes[i].pitch = pitch;
                                                mem.notes[i].start = start;
                                                mem.notes[i].length = length;
                                                mem.notes[i].velocity = velocity;
                                                break; // Stop searching once we find the right note
                                            }
                                        }
                                    }
                                    Action::DeleteNote { id } => {
                                        mem.notes.retain(|n| n.id != id);
                                    }
                                    Action::SetLockZone { start, end, .. } => {
                                        for i in start..=end {
                                            if i < 256 {
                                                mem.lock_map[i] = true;
                                            }
                                        }
                                    }
                                    Action::ClearLockZone { .. } => {
                                        mem.lock_map = [false; 256];
                                    }
                                    Action::SetRoot { root } => unsafe {
                                        let ptr =
                                            &params.root_note as *const _ as *const std::sync::atomic::AtomicI32;
                                        (*ptr).store(root, std::sync::atomic::Ordering::Relaxed);
                                    }
                                    Action::SetMode { mode } => unsafe {
                                        let ptr =
                                            &params.scale_mode as *const _ as *const std::sync::atomic::AtomicI32;
                                        (*ptr).store(mode, std::sync::atomic::Ordering::Relaxed);
                                    }
                                    Action::SetInternalBpm { bpm } => unsafe {
                                        let ptr =
                                            &params.internal_bpm as *const _ as *const std::sync::atomic::AtomicU32;
                                        // No asterisk! It's just a regular f32 now.
                                        (*ptr).store(
                                            bpm.to_bits(),
                                            std::sync::atomic::Ordering::Relaxed
                                        );
                                    }
                                    Action::ToggleSync { sync } => unsafe {
                                        let ptr =
                                            &params.sync_to_host as *const _ as *const std::sync::atomic::AtomicBool;
                                        // No asterisk! It's just a regular bool now.
                                        (*ptr).store(sync, std::sync::atomic::Ordering::Relaxed);
                                    }
                                    Action::GetPlayhead => {
                                        // Read the current step from the audio thread
                                        let step = params.active_step.load(
                                            std::sync::atomic::Ordering::Relaxed
                                        );

                                        // Fire it back to JavaScript!
                                        let payload = serde_json
                                            ::to_value(&(Event::UpdatePlayhead { step }))
                                            .unwrap();
                                        window_handler.send_json(payload);
                                    }
                                }
                            }
                        }
                    })
            )
        )
    }
}
fn initialize(
    &mut self,
    _io: &AudioIOLayout,
    _cfg: &BufferConfig,
    _ctx: &mut impl InitContext<Self>
) -> bool {
    true
}

fn reset(&mut self) {}

fn process(
    &mut self,
    _buffer: &mut Buffer,
    _aux: &mut AuxiliaryBuffers,
    context: &mut impl ProcessContext<Self>
) -> ProcessStatus {
    let transport = context.transport();

    if transport.playing {
        if let Some(pos_beats) = transport.pos_beats() {
            let current_16th = (pos_beats * 4.0) as usize;
            let step_index = current_16th % 256;

            if step_index != self.last_processed_step {
                self.last_processed_step = step_index;
                self.params.active_step.store(
                    step_index as u32,
                    std::sync::atomic::Ordering::Relaxed
                );

                for pitch in self.active_live_notes.drain(..) {
                    context.send_event(nih_plug::midi::NoteEvent::NoteOff {
                        timing: 0,
                        voice_id: None,
                        channel: 0,
                        note: pitch,
                        velocity: 0.0,
                    });
                }

                if let Ok(mem) = self.params.mem.try_lock() {
                    if mem.lock_map[step_index] {
                        for note in &mem.notes {
                            if note.start == step_index {
                                context.send_event(nih_plug::midi::NoteEvent::NoteOn {
                                    timing: 0,
                                    voice_id: Some(note.id as i32),
                                    channel: 0,
                                    note: note.pitch,
                                    velocity: (note.velocity as f32) / 127.0,
                                });
                            }
                        }
                    } else {
                        // --- THE MELODIC GRAVITY ARM ---
                        let rest_prob = self.params.rest_probability.value();
                        if fastrand::f32() > rest_prob {
                            let minor_intervals = [0, 2, 3, 5, 7, 8, 10];
                            let random_interval =
                                minor_intervals[fastrand::usize(..minor_intervals.len())];
                            let root = self.params.root_note.value();

                            let octave_offsets = [-12i32, 0, 12];
                            let random_octave =
                                octave_offsets[fastrand::usize(..octave_offsets.len())];
                            let mut final_pitch = (36 +
                                root +
                                (random_interval as i32) +
                                random_octave) as u8;

                            let current_jump = (final_pitch as i32) - (self.last_pitch as i32);
                            let same_direction =
                                (current_jump > 0 && self.cumulative_jump > 0) ||
                                (current_jump < 0 && self.cumulative_jump < 0);
                            let vol_limit = self.params.volatility.value() as i32;

                            if same_direction && self.cumulative_jump.abs() > 6 {
                                if (self.cumulative_jump + current_jump).abs() > vol_limit {
                                    final_pitch = ((self.last_pitch as i32) - current_jump) as u8;
                                    self.cumulative_jump = -current_jump;
                                } else {
                                    self.cumulative_jump += current_jump;
                                }
                            } else {
                                self.cumulative_jump = current_jump;
                            }
                            self.last_pitch = final_pitch;

                            if final_pitch >= 24 && final_pitch <= 84 {
                                context.send_event(nih_plug::midi::NoteEvent::NoteOn {
                                    timing: 0,
                                    voice_id: None,
                                    channel: 0,
                                    note: final_pitch,
                                    velocity: 0.8,
                                });
                                self.active_live_notes.push(final_pitch);
                            }
                        }
                    } // End else (Random Arm)
                } // End mem lock
            } // End step change
        } // End pos_beats
    } // End transport.playing

    ProcessStatus::Normal
}
// <--- THIS ONE BRACE FIXES EVERYTHING. It closes `impl Plugin for Mugrim`

// These must live OUTSIDE the Plugin block
impl ClapPlugin for Mugrim {
    const CLAP_ID: &'static str = "com.aaronarnold.mugrim";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Generative Hybrid Sequencer");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::NoteEffect,
        ClapFeature::Instrument,
    ];
}

impl Vst3Plugin for Mugrim {
    const VST3_CLASS_ID: [u8; 16] = *b"MuGRiMSeqExamp13";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Instrument,
        Vst3SubCategory::Tools,
    ];
}

nih_export_clap!(Mugrim);
nih_export_vst3!(Mugrim);
