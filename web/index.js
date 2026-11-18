import init, * as wasm from "./wasm.js";

// The XO-CHIP buzzer worklet is kept inline: `wasm-pack` rewrites
// `web/.gitignore` on every build, so a separate file in this directory would
// be easy to lose. The source is plain JavaScript with no template
// interpolation.
const XO_AUDIO_WORKLET = `
const FREQ = 4000;
const PITCH_BIAS = 64;
const CUTOFF = 18000;
const PATTERN_BITS = 128;
const DC_CUTOFF = 20;

function lowPassAlpha(sampleRate) {
    const c = Math.cos((2 * Math.PI * CUTOFF) / sampleRate);
    return c - 1 + Math.sqrt(c * c - 4 * c + 3);
}

class XoAudioProcessor extends AudioWorkletProcessor {
    constructor() {
        super();

        this.pattern = new Uint8Array(16);
        this.pitch = PITCH_BIAS;
        this.playing = false;

        // Fractional position in bits inside the 128-bit pattern.
        this.pos = 0;

        // Octo uses a 4-pole RC low-pass at 18 kHz and supersamples to keep
        // high playback rates from aliasing.
        this.quality = Math.max(1, Math.ceil(384000 / sampleRate));
        this.alpha = lowPassAlpha(sampleRate * this.quality);
        this.lp = [0, 0, 0, 0];

        // A gentle DC blocker keeps patterns with a constant level (and the
        // transition when the buzzer starts or stops) from producing a thump.
        this.dcR = Math.exp((-2 * Math.PI * DC_CUTOFF) / sampleRate);
        this.dcX = 0;
        this.dcY = 0;

        this.port.onmessage = (event) => {
            const data = event.data;
            if (!data) return;
            if (data.pattern) {
                this.pattern = Uint8Array.from(data.pattern);
            }
            if (typeof data.pitch === "number") {
                this.pitch = data.pitch;
            }
            if (typeof data.playing === "boolean") {
                this.playing = data.playing;
            }
        };
    }

    process(_inputs, outputs) {
        const output = outputs[0];
        if (!output || output.length === 0) return true;
        const out = output[0];

        // The spec only resets the offset when the buzzer timer reaches zero,
        // never when it is merely reassigned to another nonzero value.
        if (!this.playing) {
            this.pos = 0;
            this.lp[0] = this.lp[1] = this.lp[2] = this.lp[3] = 0;
            this.dcX = 0;
            this.dcY = 0;
            out.fill(0);
            return true;
        }

        const step = (FREQ * 2 ** ((this.pitch - PITCH_BIAS) / 48)) / sampleRate;
        const subStep = step / this.quality;

        for (let i = 0; i < out.length; i++) {
            let value = 0;
            for (let j = 0; j < this.quality; j++) {
                const bitIndex = this.pos | 0;
                const byte = this.pattern[(bitIndex >> 3) & 15];
                const bit = (byte >> (7 - (bitIndex & 7))) & 1;

                this.lp[0] = bit;
                for (let k = 1; k < 4; k++) {
                    this.lp[k] += (this.lp[k - 1] - this.lp[k]) * this.alpha;
                }
                value = this.lp[3];

                this.pos += subStep;
                if (this.pos >= PATTERN_BITS) {
                    this.pos %= PATTERN_BITS;
                }
            }

            const y = value - this.dcX + this.dcR * this.dcY;
            this.dcX = value;
            this.dcY = y;
            out[i] = y;
        }

        return true;
    }
}

registerProcessor("xo-audio", XoAudioProcessor);
`;

const WIDTH = 128;
const HEIGHT = 64;
const SCALE = 6;
const FPS_INTERVAL = 1000 / 60;
// Browsers often deliver rAF deltas a hair under 16.67ms. Without a small
// tolerance a 60Hz display alternates between 0 and 2 simulated frames.
const FRAME_EPSILON = 1;
const AUDIO_VOLUME = 0.2;

const MODE_OPTIONS = ["auto", "chip8", "octo", "schip"];
const SPEED_OPTIONS = ["0.25", "0.5", "1", "1.5", "2", "3", "4", "8", "16", "25", "custom"];
const MAX_CYCLES_PER_FRAME = 100000;

let anim_frame = 0;
let ticks_per_frame = 20;
let speed_multiplier = 1;
// When set, this exact cycles/frame value takes precedence over the speed
// multiplier. Some Octo games need far more than the default 20 cycles/frame.
let cycles_override = null;
let last_time = 0;
let frame_accum = 0;
let current_rom = null;
let current_filename = "";
let audio_ctx = null;
let audio_node = null;
let audio_ready = false;
let audio_promise = null;
let audio_worklet_url = null;
let drag_counter = 0;

const has_saved_palette = Boolean(localStorage.getItem("chip8_p1") || localStorage.getItem("chip8_fg"));
let p1_color = localStorage.getItem("chip8_p1") || localStorage.getItem("chip8_fg") || "#ffffff";
let bg_color = localStorage.getItem("chip8_bg") || "#000000";
// A fresh browser gets the Classic B&W preset; a custom foreground keeps a
// dimmed second plane and an even dimmer blend colour.
let p2_color = localStorage.getItem("chip8_p2") ||
    (has_saved_palette ? mix_hex(p1_color, bg_color, 0.5) : "#aaaaaa");
let p3_color = localStorage.getItem("chip8_p3") ||
    (has_saved_palette ? mix_hex(p1_color, bg_color, 0.15) : "#555555");

const canvas = document.getElementById("canvas");
canvas.width = WIDTH * SCALE;
canvas.height = HEIGHT * SCALE;

const ctx = canvas.getContext("2d");

const input = document.getElementById("fileinput");
const loadBtn = document.getElementById("loadBtn");
const resetBtn = document.getElementById("reset");
const settingsToggle = document.getElementById("settingsToggle");
const settingsPanel = document.getElementById("settings");
const modeSelect = document.getElementById("modeSelect");
const speedSelect = document.getElementById("speedSelect");
const cyclesInput = document.getElementById("cyclesInput");
const p1Input = document.getElementById("p1Color");
const p2Input = document.getElementById("p2Color");
const p3Input = document.getElementById("p3Color");
const bgInput = document.getElementById("bgColor");
const presetSelect = document.getElementById("themePreset");
const statusBar = document.getElementById("statusBar");

function set_settings_open(open) {
    settingsPanel.hidden = !open;
    settingsToggle.setAttribute("aria-expanded", open ? "true" : "false");
}

set_settings_open(false);
statusBar.textContent = "Load a ROM or drop it anywhere on the page.";

const CODE_TO_KEY = {
    Digit1: "1", Digit2: "2", Digit3: "3", Digit4: "4",
    KeyQ: "q",   KeyW: "w",   KeyE: "e",   KeyR: "r",
    KeyA: "a",   KeyS: "s",   KeyD: "d",   KeyF: "f",
    KeyZ: "z",   KeyX: "x",   KeyC: "c",   KeyV: "v",
};

function sync_color_inputs() {
    p1Input.value = p1_color;
    p2Input.value = p2_color;
    p3Input.value = p3_color;
    bgInput.value = bg_color;
}

function sync_preset_select() {
    const value = [p1_color, p2_color, p3_color, bg_color]
        .map((color) => color.toLowerCase())
        .join("|");
    const match = Array.from(presetSelect.options).find((opt) => opt.value === value);
    presetSelect.value = match ? match.value : "custom";
}

function clear_canvas() {
    ctx.fillStyle = bg_color;
    ctx.fillRect(0, 0, WIDTH * SCALE, HEIGHT * SCALE);
}

function save_palette() {
    localStorage.setItem("chip8_p1", p1_color);
    localStorage.setItem("chip8_p2", p2_color);
    localStorage.setItem("chip8_p3", p3_color);
    localStorage.setItem("chip8_bg", bg_color);
    // Keep the legacy keys in sync for older versions of the page.
    localStorage.setItem("chip8_fg", p1_color);
}

function update_colors(p1, p2, p3, bg) {
    p1_color = p1;
    p2_color = p2;
    p3_color = p3;
    bg_color = bg;
    sync_color_inputs();
    sync_preset_select();
    save_palette();
    clear_canvas();
}

sync_color_inputs();
sync_preset_select();
clear_canvas();

function hex_to_rgb(hex) {
    const n = parseInt(hex.replace("#", ""), 16);
    return [(n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff];
}

function mix_rgb(a, b, t) {
    return a.map((value, index) => Math.round(value * (1 - t) + b[index] * t));
}

function rgb_to_hex(rgb) {
    return `#${rgb.map((v) => v.toString(16).padStart(2, "0")).join("")}`;
}

function mix_hex(a, b, t) {
    return rgb_to_hex(mix_rgb(hex_to_rgb(a), hex_to_rgb(b), t));
}

function rgb_to_u32(rgb) {
    return (rgb[0] << 16) | (rgb[1] << 8) | rgb[2];
}

// XO-CHIP has two bitplanes plus a blend colour used where both planes are
// set. All three are directly configurable in the UI.
function palette() {
    return [
        rgb_to_u32(hex_to_rgb(p1_color)),
        rgb_to_u32(hex_to_rgb(p2_color)),
        rgb_to_u32(hex_to_rgb(p3_color)),
    ];
}

function redraw(chip8) {
    clear_canvas();
    const [c1, c2, c3] = palette();
    chip8.draw_screen(SCALE, c1, c2, c3);
}

// ---------------------------------------------------------------- audio ----

function init_audio() {
    if (audio_promise) {
        if (audio_ctx && audio_ctx.state === "suspended") {
            audio_ctx.resume().catch(() => {});
        }
        return audio_promise;
    }

    const AudioCtx = window.AudioContext || window.webkitAudioContext;
    if (!AudioCtx) {
        audio_promise = Promise.resolve(false);
        return audio_promise;
    }

    audio_ctx = new AudioCtx();
    // start_game runs inside a user gesture, which is what lets the browser
    // actually start the context.
    audio_ctx.resume().catch(() => {});

    if (!audio_ctx.audioWorklet) {
        audio_promise = Promise.resolve(false);
        return audio_promise;
    }

    if (!audio_worklet_url) {
        const blob = new Blob([XO_AUDIO_WORKLET], { type: "text/javascript" });
        audio_worklet_url = URL.createObjectURL(blob);
    }

    audio_promise = audio_ctx.audioWorklet
        .addModule(audio_worklet_url)
        .then(() => {
            audio_node = new AudioWorkletNode(audio_ctx, "xo-audio", {
                numberOfInputs: 0,
                numberOfOutputs: 1,
                outputChannelCount: [1],
            });

            const gain = audio_ctx.createGain();
            gain.gain.value = AUDIO_VOLUME;
            audio_node.connect(gain).connect(audio_ctx.destination);
            audio_ready = true;
            document.body.dataset.audio = "ready";
            return true;
        })
        .catch((error) => {
            console.error("XO-CHIP audio unavailable:", error);
            audio_ready = false;
            document.body.dataset.audio = "unavailable";
            return false;
        });

    return audio_promise;
}

function sync_audio(chip8) {
    if (!audio_ready || !audio_node) return;
    audio_node.port.postMessage({
        pattern: chip8.audio_pattern(),
        pitch: chip8.pitch(),
        playing: chip8.is_beeping(),
    });
}

// ----------------------------------------------------- per-game settings ----

function game_storage_key(name, size) {
    return `chip8_game::${name}::${size}`;
}

function read_game_settings(name, size) {
    try {
        const raw = localStorage.getItem(game_storage_key(name, size));
        if (raw) return JSON.parse(raw);
    } catch (_) {
        // Ignore corrupt or unavailable storage.
    }
    return null;
}

function apply_stored_settings(name, size) {
    const stored = read_game_settings(name, size) || {};
    const mode = MODE_OPTIONS.includes(stored.mode)
        ? stored.mode
        : localStorage.getItem("chip8_mode") || "auto";

    let speed = SPEED_OPTIONS.includes(String(stored.speed))
        ? String(stored.speed)
        : localStorage.getItem("chip8_speed") || "1";
    if (!SPEED_OPTIONS.includes(speed)) speed = "1";

    modeSelect.value = MODE_OPTIONS.includes(mode) ? mode : "auto";
    speedSelect.value = speed;
    speed_multiplier = parseFloat(speedSelect.value);
    if (!Number.isFinite(speed_multiplier)) speed_multiplier = 1;

    cycles_override = null;
    if (speed === "custom") {
        const raw = stored.cycles ?? localStorage.getItem("chip8_cycles");
        const parsed = Number(raw);
        if (Number.isFinite(parsed) && parsed > 0) {
            cycles_override = clamp_cycles(parsed);
            cyclesInput.value = String(cycles_override);
        }
    }
}

function clamp_cycles(value) {
    if (!Number.isFinite(value)) return 1;
    return Math.min(MAX_CYCLES_PER_FRAME, Math.max(1, Math.round(value)));
}

function save_game_settings() {
    const mode = modeSelect.value;
    const speed = speedSelect.value;
    localStorage.setItem("chip8_mode", mode);
    localStorage.setItem("chip8_speed", speed);
    localStorage.setItem("chip8_cycles", cycles_override == null ? "" : String(cycles_override));

    if (current_rom) {
        try {
            localStorage.setItem(
                game_storage_key(current_filename, current_rom.length),
                JSON.stringify({ mode, speed, cycles: cycles_override }),
            );
        } catch (_) {
            // Storage can be full or disabled; settings just won't persist.
        }
    }
}

// --------------------------------------------------------------- machine ----

function mode_id_from_select() {
    if (modeSelect.value === "chip8") return 0;
    if (modeSelect.value === "schip") return 2;
    return 1; // "auto" and "octo"
}

function update_timing(chip8) {
    chip8.set_mode(mode_id_from_select());

    speed_multiplier = parseFloat(speedSelect.value);
    if (!Number.isFinite(speed_multiplier)) speed_multiplier = 1;

    if (cycles_override != null) {
        ticks_per_frame = cycles_override;
    } else {
        const base = chip8.cycles_per_frame();
        ticks_per_frame = Math.max(1, Math.round(base * speed_multiplier));
        cyclesInput.value = String(ticks_per_frame);
    }
}

function update_status(chip8) {
    if (!current_rom) return;

    const modeNames = ["CHIP-8 (VIP)", "Octo / XO-CHIP", "Super-CHIP"];
    const activeMode = modeNames[chip8.get_mode()] || "CHIP-8";
    const speedText = cycles_override != null ? "custom" : `${speed_multiplier}×`;
    statusBar.textContent =
        `Loaded: ${current_filename} (${current_rom.length} bytes) - ` +
        `Mode: ${activeMode} - ${ticks_per_frame} cycles/frame (${speedText})`;
}

function apply_mode(chip8) {
    update_timing(chip8);
    update_status(chip8);
}

function load_rom_file(chip8, file) {
    if (!file) return;

    current_filename = file.name;
    const fr = new FileReader();
    fr.onload = () => {
        const data = new Uint8Array(fr.result);
        current_rom = data;
        resetBtn.disabled = false;
        apply_stored_settings(file.name, data.length);
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

    function pick_colors_and_redraw() {
        update_colors(p1Input.value, p2Input.value, p3Input.value, bgInput.value);
        if (current_rom) redraw(chip8);
    }

    p1Input.addEventListener("input", pick_colors_and_redraw);
    p2Input.addEventListener("input", pick_colors_and_redraw);
    p3Input.addEventListener("input", pick_colors_and_redraw);
    bgInput.addEventListener("input", pick_colors_and_redraw);

    presetSelect.addEventListener("change", (e) => {
        if (e.target.value === "custom") return;
        const [p1, p2, p3, bg] = e.target.value.split("|");
        update_colors(p1, p2, p3, bg);
        if (current_rom) redraw(chip8);
        presetSelect.blur();
    });

    modeSelect.addEventListener("change", () => {
        save_game_settings();
        if (current_rom) {
            start_game(chip8, current_rom);
        }
        modeSelect.blur();
    });

    speedSelect.addEventListener("change", () => {
        if (speedSelect.value === "custom") {
            // Freeze the current rate so the number field can be edited.
            cycles_override = ticks_per_frame;
            cyclesInput.value = String(cycles_override);
        } else {
            cycles_override = null;
        }

        save_game_settings();
        if (current_rom) {
            update_timing(chip8);
            // Keep a halt message visible; the program is not running anymore.
            if (!chip8.is_halted()) {
                update_status(chip8);
                redraw(chip8);
            }
        }
        speedSelect.blur();
    });

    cyclesInput.addEventListener("change", () => {
        const parsed = Number.parseInt(cyclesInput.value, 10);
        if (!Number.isFinite(parsed)) {
            cyclesInput.value = String(ticks_per_frame);
            return;
        }

        cycles_override = clamp_cycles(parsed);
        cyclesInput.value = String(cycles_override);
        speedSelect.value = "custom";

        save_game_settings();
        if (current_rom) {
            update_timing(chip8);
            if (!chip8.is_halted()) {
                update_status(chip8);
                redraw(chip8);
            }
        }
        cyclesInput.blur();
    });

    document.addEventListener("keydown", (evt) => {
        if (evt.key === "Escape" && !settingsPanel.hidden) {
            set_settings_open(false);
            settingsToggle.focus();
            return;
        }

        const tag = evt.target?.tagName;
        if (tag === "INPUT" || tag === "SELECT" || tag === "TEXTAREA") {
            // Let the user type a cycles/frame value or use the controls.
            return;
        }

        if (evt.code === "Backspace" && current_rom) {
            evt.preventDefault();
            start_game(chip8, current_rom);
            return;
        }
        handle_key(chip8, evt, true);
    });

    document.addEventListener("keyup", (evt) => {
        const tag = evt.target?.tagName;
        if (tag === "INPUT" || tag === "SELECT" || tag === "TEXTAREA") {
            return;
        }
        handle_key(chip8, evt, false);
    });

    // Audio contexts start suspended in some browsers until a gesture.
    document.addEventListener("pointerdown", () => {
        if (audio_ctx && audio_ctx.state === "suspended") {
            audio_ctx.resume().catch(() => {});
        }
    });

    settingsToggle.addEventListener("click", () => {
        set_settings_open(settingsPanel.hidden);
    });

    loadBtn.addEventListener("click", () => {
        input.click();
    });

    resetBtn.addEventListener("click", () => {
        if (current_rom) {
            start_game(chip8, current_rom);
        }
        resetBtn.blur();
    });

    input.addEventListener("change", (evt) => {
        const file = evt.target.files[0];
        // Clear the field so picking the same ROM again still fires `change`.
        input.value = "";
        load_rom_file(chip8, file);
        input.blur();
    }, false);
}

function start_game(chip8, rom) {
    init_audio();
    if (anim_frame !== 0) {
        window.cancelAnimationFrame(anim_frame);
        anim_frame = 0;
    }

    // load_game resets the machine internally and keeps the selected profile.
    const ok = chip8.load_game(rom);
    if (!ok) {
        statusBar.textContent = `Error: Invalid or oversized ROM (${rom.length} bytes)`;
        return;
    }

    apply_mode(chip8);
    clear_canvas();
    sync_audio(chip8);

    last_time = performance.now();
    frame_accum = 0;
    mainloop(chip8, last_time);
}

function mainloop(chip8, current_time = 0) {
    anim_frame = window.requestAnimationFrame((t) => mainloop(chip8, t));

    if (chip8.is_halted()) {
        const reason = chip8.halt_reason();
        statusBar.textContent = `Halted: ${reason || "unknown reason"}`;
        if (audio_ready && audio_node) {
            audio_node.port.postMessage({ playing: false });
        }
        window.cancelAnimationFrame(anim_frame);
        anim_frame = 0;
        return;
    }

    if (!last_time) last_time = current_time;
    let elapsed = current_time - last_time;
    if (elapsed < 0) elapsed = 0;
    if (elapsed > 250) elapsed = 250;
    last_time = current_time;
    frame_accum += elapsed;

    let steps = 0;
    while (frame_accum >= FPS_INTERVAL - FRAME_EPSILON && steps < 4) {
        for (let i = 0; i < ticks_per_frame; i++) {
            chip8.tick();
            if (chip8.take_vblank()) break;
        }
        chip8.tick_timers();
        frame_accum -= FPS_INTERVAL;
        steps++;
        if (chip8.is_halted()) break;
    }

    if (steps === 0) {
        return;
    }

    sync_audio(chip8);
    redraw(chip8);
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
