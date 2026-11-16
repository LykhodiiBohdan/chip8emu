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

// Загружаем сохранённые цвета или ставим классические чёрно-белые
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

async function run() {
    await init();
    const chip8 = new wasm.EmuWasm();

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
        if (evt.key === "Backspace" && current_rom) {
            evt.preventDefault();
            start_game(chip8, current_rom);
            return;
        }
        chip8.keypress(evt, true);
    });

    document.addEventListener("keyup", (evt) => {
        chip8.keypress(evt, false);
    });

    resetBtn.addEventListener("click", () => {
        if (current_rom) {
            start_game(chip8, current_rom);
        }
        resetBtn.blur();
    });

    input.addEventListener("change", (evt) => {
        const file = evt.target.files[0];
        if (!file) {
            return;
        }

        const fr = new FileReader();
        fr.onload = () => {
            current_rom = new Uint8Array(fr.result);
            resetBtn.disabled = false;
            start_game(chip8, current_rom);
        };
        fr.readAsArrayBuffer(file);
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

run().catch(console.error);
