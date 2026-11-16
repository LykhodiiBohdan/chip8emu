import init, * as wasm from "./wasm.js";

const WIDTH = 64;
const HEIGHT = 32;
const SCALE = 15;
const TICKS_PER_FRAME = 15;
const FPS_INTERVAL = 1000 / 60;

let anim_frame = 0;
let last_time = 0;
let current_rom = null;
let audio_ctx = null;
let gain_node = null;
let drag_counter = 0;

let fg_color = localStorage.getItem("chip8_fg") || "#ffffff";
let bg_color = localStorage.getItem("chip8_bg") || "#000000";

const canvas = document.getElementById("canvas");
canvas.width = WIDTH * SCALE;
canvas.height = HEIGHT * SCALE;

const ctx = canvas.getContext("2d");

const input = document.getElementById("fileinput");
const resetBtn = document.getElementById("reset");
const fgInput = document.getElementById("fgColor");
const bgInput = document.getElementById("bgColor");
const presetSelect = document.getElementById("themePreset");
const statusBar = document.getElementById("statusBar");

const CODE_TO_KEY = {
    Digit1: "1", Digit2: "2", Digit3: "3", Digit4: "4",
    KeyQ: "q",   KeyW: "w",   KeyE: "e",   KeyR: "r",
    KeyA: "a",   KeyS: "s",   KeyD: "d",   KeyF: "f",
    KeyZ: "z",   KeyX: "x",   KeyC: "c",   KeyV: "v",
};

fgInput.value = fg_color;
bgInput.value = bg_color;

function clear_canvas() {
    ctx.fillStyle = bg_color;
    ctx.fillRect(0, 0, WIDTH * SCALE, HEIGHT * SCALE);
}

function update_colors(fg, bg) {
    fg_color = fg;
    bg_color = bg;
    fgInput.value = fg;
    bgInput.value = bg;
    localStorage.setItem("chip8_fg", fg);
    localStorage.setItem("chip8_bg", bg);
    clear_canvas();
}

clear_canvas();

function init_audio() {
    if (audio_ctx) return;
    const AudioCtx = window.AudioContext || window.webkitAudioContext;
    audio_ctx = new AudioCtx();

    const oscillator = audio_ctx.createOscillator();
    gain_node = audio_ctx.createGain();

    oscillator.type = "square";
    oscillator.frequency.setValueAtTime(440, audio_ctx.currentTime);
    gain_node.gain.setValueAtTime(0, audio_ctx.currentTime);

    oscillator.connect(gain_node);
    gain_node.connect(audio_ctx.destination);
    oscillator.start();
}

function load_rom_file(chip8, file) {
    if (!file) return;

    const fr = new FileReader();
    fr.onload = () => {
        current_rom = new Uint8Array(fr.result);
        resetBtn.disabled = false;
        statusBar.textContent = `Loaded: ${file.name} (${current_rom.length} bytes)`;
        start_game(chip8, current_rom);
    };
    fr.readAsArrayBuffer(file);
}

async function run() {
    await init();
    const chip8 = new wasm.EmuWasm();


    window.addEventListener("dragenter", (e) => {
        e.preventDefault();
        drag_counter++;
        document.body.classList.add("drag-over");
    });

    window.addEventListener("dragover", (e) => {
        e.preventDefault();
        if (e.dataTransfer) {
            e.dataTransfer.dropEffect = "copy";
        }
    });

    window.addEventListener("dragleave", (e) => {
        e.preventDefault();
        drag_counter--;
        if (drag_counter <= 0) {
            drag_counter = 0;
            document.body.classList.remove("drag-over");
        }
    });

    window.addEventListener("drop", (e) => {
        e.preventDefault();
        drag_counter = 0;
        document.body.classList.remove("drag-over");

        const file = e.dataTransfer?.files?.[0];
        if (file) {
            input.value = "";
            load_rom_file(chip8, file);
        }
    });

    fgInput.addEventListener("input", (e) => {
        presetSelect.value = "custom";
        update_colors(e.target.value, bg_color);
        if (current_rom) {
            ctx.fillStyle = fg_color;
            chip8.draw_screen(SCALE);
        }
    });

    bgInput.addEventListener("input", (e) => {
        presetSelect.value = "custom";
        update_colors(fg_color, e.target.value);
        if (current_rom) {
            ctx.fillStyle = fg_color;
            chip8.draw_screen(SCALE);
        }
    });

    presetSelect.addEventListener("change", (e) => {
        if (e.target.value === "custom") return;
        const [fg, bg] = e.target.value.split("|");
        update_colors(fg, bg);
        if (current_rom) {
            ctx.fillStyle = fg_color;
            chip8.draw_screen(SCALE);
        }
        presetSelect.blur();
    });

    document.addEventListener("keydown", (evt) => {
            if (evt.code === "Backspace" && current_rom) {
                evt.preventDefault();
                start_game(chip8, current_rom);
                return;
            }
            handle_key(chip8, evt, true);
        });

        document.addEventListener("keyup", (evt) => {
            handle_key(chip8, evt, false);
        });

    resetBtn.addEventListener("click", () => {
        if (current_rom) {
            start_game(chip8, current_rom);
        }
        resetBtn.blur();
    });

    input.addEventListener("change", (evt) => {
        const file = evt.target.files[0];
        load_rom_file(chip8, file);
        input.blur();
    }, false);
}

function start_game(chip8, rom) {
    init_audio();
    if (anim_frame !== 0) {
        window.cancelAnimationFrame(anim_frame);
    }

    chip8.reset();
    chip8.load_game(rom);
    clear_canvas();

    last_time = performance.now();
    mainloop(chip8, last_time);
}

function mainloop(chip8, current_time = 0) {
    anim_frame = window.requestAnimationFrame((t) => mainloop(chip8, t));

    const elapsed = current_time - last_time;
    if (elapsed < FPS_INTERVAL) {
        return;
    }
    last_time = current_time - (elapsed % FPS_INTERVAL);

    for (let i = 0; i < TICKS_PER_FRAME; i++) {
        chip8.tick();
    }
    chip8.tick_timers();

    if (gain_node && audio_ctx) {
        const volume = chip8.is_beeping() ? 0.05 : 0;
        gain_node.gain.setValueAtTime(volume, audio_ctx.currentTime);
    }

    clear_canvas();

    ctx.fillStyle = fg_color;
    chip8.draw_screen(SCALE);
}

function handle_key(chip8, evt, pressed) {
    const mapped = CODE_TO_KEY[evt.code];
    if (mapped) {
        chip8.keypress({ key: mapped }, pressed);
    } else {
        chip8.keypress(evt, pressed);
    }
}

run().catch(console.error);
