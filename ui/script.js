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