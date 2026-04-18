function generatePianoRoll() {
    const container = document.getElementById('piano-roll-container');
    
    // Based on your Rust default parameters (High to Low so high notes are at the top)
    const maxPitch = 52; 
    const minPitch = 30; 
    const totalSteps = 256;

    for (let pitch = maxPitch; pitch >= minPitch; pitch--) {
        // Create a row for each pitch
        const row = document.createElement('div');
        row.className = 'piano-row';
        row.dataset.pitch = pitch;

        for (let step = 0; step < totalSteps; step++) {
            // Create the individual 16th-note cell
            const key = document.createElement('div');
            key.className = 'piano-key';
            
            // This is the critical anchor for your playhead!
            key.dataset.step = step; 
            key.dataset.pitch = pitch;

            row.appendChild(key);
        }
        
        container.appendChild(row);
    }
}

// Run this immediately when the UI loads
generatePianoRoll();

// Attach ONE listener to the entire grid container
const container = document.getElementById('piano-roll-container');

container.addEventListener('click', (event) => {
    // Make sure they actually clicked a piano key cell
    if (event.target.classList.contains('piano-key')) {
        
        const step = parseInt(event.target.dataset.step, 10);
        const pitch = parseInt(event.target.dataset.pitch, 10);

        // CHECK: Is the note already drawn?
        if (event.target.classList.contains('drawn-note')) {
            
            // --- 1. DELETE THE NOTE ---
            // Retrieve the ID we stored when we created it
            const noteId = parseInt(event.target.dataset.noteId, 10);

            const deletePayload = {
                type: "DeleteNote",
                id: noteId
            };

            if (window.ipc) {
                window.ipc.postMessage(JSON.stringify(deletePayload));
            }

            // Visually clear the UI and wipe the stored ID
            event.target.classList.remove('drawn-note');
            delete event.target.dataset.noteId;

        } else {
            
            // --- 2. ADD THE NOTE ---
            const newId = Date.now(); // Generate a fresh unique ID

            const addPayload = {
                type: "AddNote",
                id: newId,
                pitch: pitch,
                start: step,
                length: 1,      // 16th note default
                velocity: 100   // Default MIDI velocity
            };

            if (window.ipc) {
                window.ipc.postMessage(JSON.stringify(addPayload));
            }

            // Paint it pink and stash the ID in the DOM for later deletion
            event.target.classList.add('drawn-note');
            event.target.dataset.noteId = newId;
        }
    }
});

// --- 1. CORE STATE & SETTINGS ---
let notes = []; 
let nextNoteId = 1;
let stepWidth = 20; 
let rowHeight = 16; 
const totalSteps = 256; 
const minPitch = 24; 
const maxPitch = 72; 
const totalPitches = maxPitch - minPitch + 1;

// UI Elements (Cached for performance)
const scrollArea = document.getElementById('scroll-area');
const gridContent = document.getElementById('grid-content');
const gridBg = document.getElementById('grid-bg');
const pianoKeys = document.getElementById('piano-keys');
const lockTrack = document.getElementById('lock-track');
const wheelVelocityChk = document.getElementById('wheel-velocity-chk');

// Rust Communication Handshake
function sendToRust(actionObj) {
    fetch('mugrim://action', {
        method: 'POST',
        body: JSON.stringify(actionObj)
    }).catch(err => console.error("UI Communication Error:", err));
}

// ... Paste your renderGrid(), createNoteElement(), and event listeners here ...

// Initialize
renderGrid();


// 1. The Engine Ping
function startPlayheadPolling() {
    if (window.ipc) {
        window.ipc.postMessage(JSON.stringify({ type: "GetPlayhead" }));
    }
    // Loop it at the monitor's refresh rate
    requestAnimationFrame(startPlayheadPolling); 
}

// Start the engine
startPlayheadPolling();

// 2. The Receiver
window.addEventListener("message", (event) => {
    // Safely parse the message whether it's an object or string
    const msg = typeof event.data === 'string' ? JSON.parse(event.data) : event.data;
    
    if (msg.type === "UpdatePlayhead") {
        const currentStep = msg.step;
        
        // 3. The Visual Update
        // Wipe the glow from the previous frame
        document.querySelectorAll('.piano-key').forEach(key => {
            key.classList.remove('glow-active');
        });
        
        // Find notes that start at the exact current 16th note step
        // (Assuming your DOM elements have a data-step attribute)
        const activeKeys = document.querySelectorAll(`.piano-key[data-step="${currentStep}"]`);
        activeKeys.forEach(key => key.classList.add('glow-active'));
        
    } else if (msg.type === "SyncState") {
        const notes = msg.notes;
        
        // Loop through the notes sent by Rust
        notes.forEach(note => {
            // Find the exact UI cell using our coordinate matrix
            const key = document.querySelector(`.piano-key[data-step="${note.start}"][data-pitch="${note.pitch}"]`);
            
            if (key) {
                // Paint it and restore the unique ID so it can still be deleted
                key.classList.add('drawn-note');
                key.dataset.noteId = note.id;
            }
        });
    }
});

// Ask Rust for the saved notes 100ms after the UI loads
    setTimeout(() => {
        if (window.ipc) {
            window.ipc.postMessage(JSON.stringify({ type: "RequestSync" }));
        }
    }, 100);

    // Get the buttons we named in the HTML
const toSequencerBtn = document.getElementById('btn-to-sequencer');
const toMainBtn = document.getElementById('btn-to-main');

// Get the view containers
const mainView = document.getElementById('main-view');
const sequencerView = document.getElementById('sequencer-view');

toSequencerBtn.onclick = () => {
    mainView.style.display = 'none';
    sequencerView.style.display = 'block';
};

toMainBtn.onclick = () => {
    mainView.style.display = 'block';
    sequencerView.style.display = 'none';
};

const sliders = ['rest_prob', 'min_pitch', 'max_pitch', 'min_note_length', 'max_note_length']; 

sliders.forEach(id => {
    const el = document.getElementById(id);
    if (el) {
        el.addEventListener('input', (e) => {
            window.ipc.postMessage(JSON.stringify({
                type: "SetParameter", // Make sure this matches your Rust Action enum!
                param: id,
                value: parseFloat(e.target.value) / 100 
            }));
        });
    }
});

const overlapToggle = document.getElementById('allow_note_overlap');
overlapToggle.addEventListener('change', (e) => {
    window.ipc.postMessage(JSON.stringify({
        type: "ToggleOverlap",
        value: e.target.checked
    }));
});