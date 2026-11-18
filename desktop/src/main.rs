//! SDL2 desktop host.
//!
//! The window shows a crisp 128x64 CHIP-8 screen (integer-scaled with a
//! nearest-neighbour texture) plus a settings sheet that mirrors the web UI:
//! emulation profile, speed/cycles, palette channels and presets.

mod font;
mod settings;
mod ui;

use chip8_core::*;
use sdl2::audio::{AudioCallback, AudioSpecDesired};
use sdl2::event::{Event, WindowEvent};
use sdl2::keyboard::Keycode;
use sdl2::mouse::MouseButton;
use sdl2::pixels::{Color, PixelFormatEnum};
use sdl2::rect::Rect;
use sdl2::render::{BlendMode, ScaleMode, TextureCreator};
use sdl2::video::{FullscreenType, WindowContext};
use settings::{Channel, MODE_NAMES, PRESETS, Palette, Settings, Speed};
use std::env;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use ui::{ACCENT, FIELD, MUTED, PANEL, PANEL_BORDER, TEXT, Ui};

const WINDOW_WIDTH: u32 = 1280;
const WINDOW_HEIGHT: u32 = 720;

/// One emulated CHIP-8 frame is always 1/60 s, independent of the display
/// refresh rate. Presentation is paced by vsync; emulation is paced by an
/// accumulator so 144 Hz windows do not run the game 2.4x too fast.
const FRAME_STEP: f32 = 1.0 / 60.0;

const BACKDROP: Color = Color::RGB(16, 16, 18);

const AUDIO_PATTERN_BITS: f32 = 128.0;
const AUDIO_BASE_FREQ: f32 = 4000.0;
const AUDIO_PITCH_BIAS: f32 = 64.0;
const AUDIO_VOLUME: f32 = 0.2;
const LOW_PASS_CUTOFF: f32 = 18_000.0;
const DC_CUTOFF: f32 = 20.0;

#[derive(Clone)]
struct AudioState {
    pattern: [u8; 16],
    pitch: u8,
    playing: bool,
}

struct XoBuzzer {
    shared: Arc<Mutex<AudioState>>,
    sample_rate: f32,
    /// Number of filter sub-steps per output sample. Keeps playback rates
    /// above the output sample rate from aliasing, like Octo does.
    quality: u32,
    alpha: f32,
    /// Fractional position inside the 128-bit pattern.
    pos: f32,
    lp: [f32; 4],
    dc_x: f32,
    dc_y: f32,
    dc_r: f32,
}

impl XoBuzzer {
    fn new(shared: Arc<Mutex<AudioState>>, sample_rate: f32) -> Self {
        let quality = (384_000.0 / sample_rate).ceil().max(1.0) as u32;
        Self {
            shared,
            sample_rate,
            quality,
            alpha: low_pass_alpha(sample_rate * quality as f32),
            pos: 0.0,
            lp: [0.0; 4],
            dc_x: 0.0,
            dc_y: 0.0,
            dc_r: (-2.0 * std::f32::consts::PI * DC_CUTOFF / sample_rate).exp(),
        }
    }
}

fn low_pass_alpha(sample_rate: f32) -> f32 {
    let c = (2.0 * std::f32::consts::PI * LOW_PASS_CUTOFF / sample_rate).cos();
    c - 1.0 + (c * c - 4.0 * c + 3.0).sqrt()
}

impl AudioCallback for XoBuzzer {
    type Channel = f32;

    fn callback(&mut self, out: &mut [f32]) {
        let state = match self.shared.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        // XO-CHIP only resets the pattern offset when the sound timer is zero.
        if !state.playing {
            self.pos = 0.0;
            self.lp = [0.0; 4];
            self.dc_x = 0.0;
            self.dc_y = 0.0;
            out.fill(0.0);
            return;
        }

        let pitch = (state.pitch as f32 - AUDIO_PITCH_BIAS) / 48.0;
        let step = AUDIO_BASE_FREQ * pitch.exp2() / self.sample_rate;
        let sub_step = step / self.quality as f32;

        for sample in out.iter_mut() {
            let mut value = 0.0;
            for _ in 0..self.quality {
                let bit_index = self.pos as usize;
                let byte = state.pattern[(bit_index >> 3) & 15];
                let bit = ((byte >> (7 - (bit_index & 7))) & 1) as f32;

                self.lp[0] = bit;
                for k in 1..4 {
                    self.lp[k] += (self.lp[k - 1] - self.lp[k]) * self.alpha;
                }
                value = self.lp[3];

                self.pos += sub_step;
                if self.pos >= AUDIO_PATTERN_BITS {
                    self.pos %= AUDIO_PATTERN_BITS;
                }
            }

            // Remove the DC component so constant patterns stay silent and
            // starting/stopping the buzzer does not thump.
            let y = value - self.dc_x + self.dc_r * self.dc_y;
            self.dc_x = value;
            self.dc_y = y;
            *sample = y * AUDIO_VOLUME;
        }
    }
}

struct Rom {
    name: String,
    path: String,
    data: Vec<u8>,
}

impl Rom {
    fn load(emu: &mut Emu, path: &str, settings: &Settings) -> Result<Self, String> {
        let data = fs::read(path).map_err(|error| format!("{path}: {error}"))?;
        if data.is_empty() {
            return Err(format!("{path}: the file is empty"));
        }
        if data.len() > MAX_ROM_SIZE {
            return Err(format!(
                "{path}: {} bytes, the maximum is {MAX_ROM_SIZE}",
                data.len()
            ));
        }

        emu.set_mode(settings.mode);
        if !emu.load(&data) {
            return Err(format!("{path}: the ROM could not be loaded"));
        }

        let name = Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string());

        Ok(Self {
            name,
            path: path.to_string(),
            data,
        })
    }

    /// Restarts from the buffer already in memory.
    fn restart(&self, emu: &mut Emu) {
        emu.load(&self.data);
    }

    /// Re-reads the file from disk, picking up rebuilds.
    fn reload(&mut self, emu: &mut Emu, settings: &Settings) -> Result<(), String> {
        let data = fs::read(&self.path).map_err(|error| format!("{}: {error}", self.path))?;
        if data.is_empty() || data.len() > MAX_ROM_SIZE || !emu.load(&data) {
            return Err(format!("{}: the ROM could not be reloaded", self.path));
        }
        self.data = data;
        emu.set_mode(settings.mode);
        self.restart(emu);
        Ok(())
    }
}

fn main() {
    let capture = env::var("CHIP8_CAPTURE").ok();
    let capture_menu = env::var("CHIP8_CAPTURE_MENU").is_ok_and(|value| value == "1");
    let capture_frames: u32 = env::var("CHIP8_CAPTURE_FRAMES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(60);

    if capture.is_some() {
        // Offscreen rendering for screenshots. Safe here: nothing else runs yet.
        unsafe {
            env::set_var("SDL_VIDEODRIVER", "dummy");
        }
    }

    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() > 3 {
        println!("Usage: desktop [rom] [mode: 0=vip,1=octo,2=schip] [cycles/frame]");
        return;
    }

    let sdl_context = sdl2::init().unwrap();
    let video = sdl_context.video().unwrap();
    let window = video
        .window("Chip-8 Emulator", WINDOW_WIDTH, WINDOW_HEIGHT)
        .position_centered()
        .resizable()
        .build()
        .unwrap();

    let mut canvas = if capture.is_some() {
        window.into_canvas().software().build().unwrap()
    } else {
        window.into_canvas().present_vsync().build().unwrap()
    };
    canvas.set_blend_mode(BlendMode::Blend);
    let _ = canvas.window_mut().set_minimum_size(1000, 560);

    let texture_creator: TextureCreator<WindowContext> = canvas.texture_creator();
    let mut screen = texture_creator
        .create_texture_streaming(
            PixelFormatEnum::RGBA32,
            SCREEN_WIDTH as u32,
            SCREEN_HEIGHT as u32,
        )
        .unwrap();
    screen.set_scale_mode(ScaleMode::Nearest);

    let audio_shared = Arc::new(Mutex::new(AudioState {
        pattern: DEFAULT_AUDIO_PATTERN,
        pitch: 64,
        playing: false,
    }));

    let audio_subsystem = sdl_context.audio().unwrap();
    let desired_spec = AudioSpecDesired {
        freq: Some(44_100),
        channels: Some(1),
        samples: None,
    };
    let audio_device = audio_subsystem
        .open_playback(None, &desired_spec, |spec| {
            XoBuzzer::new(Arc::clone(&audio_shared), spec.freq as f32)
        })
        .unwrap();
    audio_device.resume();

    let mut event_pump = sdl_context.event_pump().unwrap();

    let mut settings = Settings::load();
    if capture_menu {
        settings.paused = false;
    }
    if let Some(mode) = args.get(1).and_then(|value| value.parse::<u8>().ok())
        && mode <= 2
    {
        settings.mode = mode;
    }
    if let Some(cycles) = args.get(2).and_then(|value| value.parse::<u32>().ok()) {
        settings.speed = Speed::Custom;
        settings.cycles = Some(Settings::clamp_cycles(cycles));
    }

    let mut emu = Emu::default();
    emu.set_mode(settings.mode);

    let mut rom = None;
    let mut message = String::new();
    if let Some(path) = args.first() {
        match Rom::load(&mut emu, path, &settings) {
            Ok(loaded) => {
                let _ = canvas
                    .window_mut()
                    .set_title(&format!("{} - Chip-8 Emulator", loaded.name));
                rom = Some(loaded);
            }
            Err(error) => message = error,
        }
    }

    let mut pixels = vec![0u8; SCREEN_WIDTH * SCREEN_HEIGHT * 4];
    let mut mouse = (0, 0);
    let mut pressed = false;
    let mut menu_open = capture_menu;
    let mut fullscreen = false;
    let mut accumulator = 0.0f32;
    let mut last_tick = Instant::now();
    let mut emu_frames = 0u32;
    let mut fps_timer = Instant::now();
    let mut fps = 0u32;
    let mut captured = 0u32;

    'gameloop: loop {
        let mut clicked = false;

        for event in event_pump.poll_iter() {
            match event {
                Event::Quit { .. } => break 'gameloop,
                Event::KeyDown {
                    keycode: Some(key),
                    repeat: false,
                    ..
                } => match key {
                    Keycode::Escape => {
                        if menu_open {
                            menu_open = false;
                            settings.save();
                        } else {
                            break 'gameloop;
                        }
                    }
                    Keycode::F1 | Keycode::Tab => menu_open = !menu_open,
                    Keycode::F11 => settings.fullscreen = !settings.fullscreen,
                    Keycode::Space => settings.paused = !settings.paused,
                    Keycode::Backspace => {
                        if let Some(current) = &rom {
                            current.restart(&mut emu);
                            message.clear();
                        }
                    }
                    key if !menu_open => {
                        if let Some(button) = key2btn(key) {
                            emu.keypress(button, true);
                        }
                    }
                    _ => {}
                },
                Event::KeyUp {
                    keycode: Some(key), ..
                } => {
                    if let Some(button) = key2btn(key) {
                        emu.keypress(button, false);
                    }
                }
                Event::Window {
                    win_event: WindowEvent::FocusLost,
                    ..
                } => {
                    // The OS stops sending key-ups once focus moves away.
                    pressed = false;
                    for button in 0..16 {
                        emu.keypress(button, false);
                    }
                }
                Event::Window {
                    win_event: WindowEvent::Leave,
                    ..
                } => {
                    // A release outside the window never reaches us.
                    pressed = false;
                }
                Event::MouseMotion { x, y, .. } => mouse = (x, y),
                Event::MouseButtonDown {
                    mouse_btn: MouseButton::Left,
                    x,
                    y,
                    ..
                } => {
                    mouse = (x, y);
                    pressed = true;
                }
                Event::MouseButtonUp {
                    mouse_btn: MouseButton::Left,
                    x,
                    y,
                    ..
                } => {
                    mouse = (x, y);
                    clicked = pressed;
                    pressed = false;
                }
                Event::DropFile { filename, .. } => match Rom::load(&mut emu, &filename, &settings)
                {
                    Ok(loaded) => {
                        let _ = canvas
                            .window_mut()
                            .set_title(&format!("{} - Chip-8 Emulator", loaded.name));
                        rom = Some(loaded);
                        message.clear();
                        menu_open = false;
                    }
                    Err(error) => message = error,
                },
                _ => {}
            }
        }

        if settings.fullscreen != fullscreen {
            let target = if settings.fullscreen {
                FullscreenType::Desktop
            } else {
                FullscreenType::Off
            };
            if canvas.window_mut().set_fullscreen(target).is_ok() {
                fullscreen = settings.fullscreen;
            } else {
                settings.fullscreen = fullscreen;
            }
        }

        let running = rom.is_some() && !settings.paused && !menu_open;

        // Screenshot mode is not real time: advance exactly one frame per
        // iteration so captures stay deterministic.
        let elapsed = if capture.is_some() {
            FRAME_STEP
        } else {
            let now = Instant::now();
            let elapsed = now.duration_since(last_tick).as_secs_f32();
            last_tick = now;
            elapsed
        };
        // Never fast-forward more than a quarter second after a stall.
        accumulator = (accumulator + elapsed).min(0.25);

        if running && !emu.is_halted() {
            while accumulator >= FRAME_STEP * 0.9 {
                accumulator -= FRAME_STEP;
                emulate_frame(&mut emu, &settings);
                emu_frames += 1;
                if emu.is_halted() {
                    message = match emu.halt_reason() {
                        Some(reason) => format!("Halted: {reason:?}"),
                        None => "Halted".to_string(),
                    };
                    accumulator = 0.0;
                    break;
                }
            }

            if let Ok(mut state) = audio_shared.lock() {
                state.pattern = *emu.audio_pattern();
                state.pitch = emu.pitch();
                state.playing = emu.is_beeping() && !emu.is_halted();
            }
        } else {
            accumulator = 0.0;
            if let Ok(mut state) = audio_shared.lock() {
                // Pausing or opening the menu must silence a running buzzer.
                state.playing = false;
            }
        }

        // ------------------------------------------------------------ draw ----
        canvas.set_draw_color(BACKDROP);
        canvas.clear();

        let (window_width, window_height) = canvas.output_size().unwrap();
        let game = game_rect(window_width, window_height);

        fill_pixels(&emu, &settings.palette, &mut pixels);
        let _ = screen.update(None, &pixels, SCREEN_WIDTH * 4);
        let _ = canvas.copy(&screen, None, Some(game));

        let base_cycles = base_cycles(settings.mode);
        let mut ui = Ui::new(&mut canvas, mouse, pressed, clicked);

        if rom.is_none() {
            ui.centered_text(
                Rect::new(
                    game.x(),
                    game.y() + game.height() as i32 / 2 - 40,
                    game.width(),
                    24,
                ),
                3,
                TEXT,
                "Drop a ROM here",
            );
            ui.centered_text(
                Rect::new(
                    game.x(),
                    game.y() + game.height() as i32 / 2,
                    game.width(),
                    16,
                ),
                2,
                MUTED,
                "or pass it on the command line, F1 for settings",
            );
        }

        draw_status_bar(
            &mut ui,
            window_width,
            window_height,
            &game,
            rom.as_ref(),
            &settings,
            base_cycles,
            fps,
            &message,
        );

        if menu_open {
            let before_mode = settings.mode;
            let mut outcome = MenuOutcome::default();
            draw_menu(
                &mut ui,
                &mut settings,
                rom.as_ref(),
                base_cycles,
                &mut outcome,
            );

            if outcome.close {
                menu_open = false;
                settings.save();
            }
            if settings.mode != before_mode {
                emu.set_mode(settings.mode);
                if let Some(current) = &rom {
                    current.restart(&mut emu);
                }
                message.clear();
            }
            if outcome.reset
                && let Some(current) = &rom
            {
                current.restart(&mut emu);
                message.clear();
            }
            if outcome.reload
                && let Some(current) = &mut rom
            {
                match current.reload(&mut emu, &settings) {
                    Ok(()) => message.clear(),
                    Err(error) => message = error,
                }
            }
        }

        canvas.present();

        // ----------------------------------------------------------- timing ----
        // Count emulated frames, not presentations: the window may refresh at 144 Hz.
        if fps_timer.elapsed() >= Duration::from_millis(500) {
            fps = (emu_frames as f64 / fps_timer.elapsed().as_secs_f64()).round() as u32;
            emu_frames = 0;
            fps_timer = Instant::now();
        }

        if let Some(path) = &capture {
            captured += 1;
            if captured >= capture_frames {
                let (width, height) = canvas.output_size().unwrap();
                let pixels = canvas.read_pixels(None, PixelFormatEnum::ARGB8888).unwrap();
                write_bmp(path, width, height, &pixels).unwrap();
                break 'gameloop;
            }
        }
    }

    audio_device.pause();
    settings.save();
}

/// Runs one 1/60 s emulated frame: a full instruction budget plus one timer tick.
fn emulate_frame(emu: &mut Emu, settings: &Settings) {
    let cycles = settings.effective_cycles(emu.cycles_per_frame());
    for _ in 0..cycles {
        emu.tick();
        if emu.take_vblank() {
            break;
        }
    }
    emu.tick_timers();
}

/// Integer-scaled, centred game viewport. Any leftover space is letterboxing.
fn game_rect(window_width: u32, window_height: u32) -> Rect {
    let scale = (window_width / SCREEN_WIDTH as u32)
        .min(window_height / SCREEN_HEIGHT as u32)
        .max(1);
    let width = SCREEN_WIDTH as u32 * scale;
    let height = SCREEN_HEIGHT as u32 * scale;
    Rect::new(
        ((window_width - width) / 2) as i32,
        ((window_height - height) / 2) as i32,
        width,
        height,
    )
}

fn base_cycles(mode: u8) -> u32 {
    let mut probe = Emu::default();
    probe.set_mode(mode);
    probe.cycles_per_frame()
}

/// Writes a 24-bit BMP (bottom-up) from `ARGB8888` renderer pixels, whose
/// little-endian byte order is already B, G, R, A.
fn write_bmp(path: &str, width: u32, height: u32, pixels: &[u8]) -> std::io::Result<()> {
    let row_bytes = width as usize * 3;
    let padding = (4 - row_bytes % 4) % 4;
    let image_size = (row_bytes + padding) * height as usize;
    let file_size = 54 + image_size;

    let mut out = Vec::with_capacity(file_size);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(file_size as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(image_size as u32).to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());

    for y in (0..height as usize).rev() {
        let start = y * width as usize * 4;
        let row = &pixels[start..start + width as usize * 4];
        for pixel in row.chunks_exact(4) {
            out.extend_from_slice(&pixel[0..3]);
        }
        out.extend(std::iter::repeat_n(0u8, padding));
    }

    fs::write(path, out)
}

/// Writes the XO-CHIP planes into an RGBA buffer for one texture upload.
fn fill_pixels(emu: &Emu, palette: &Palette, out: &mut [u8]) {
    let colors = [
        [palette.p1.r, palette.p1.g, palette.p1.b, 0xff],
        [palette.p2.r, palette.p2.g, palette.p2.b, 0xff],
        [palette.p3.r, palette.p3.g, palette.p3.b, 0xff],
    ];

    out.fill(0);
    let plane0 = emu.get_plane(0);
    let plane1 = emu.get_plane(1);
    for (index, (low, high)) in plane0.iter().zip(plane1).enumerate() {
        let pixel = *low as usize | ((*high as usize) << 1);
        if pixel == 0 {
            continue;
        }
        let offset = index * 4;
        out[offset..offset + 4].copy_from_slice(&colors[pixel - 1]);
    }

    // Transparent pixels must show the palette background, and the texture is
    // blitted over the backdrop, so paint the background colour first.
    let bg = [palette.bg.r, palette.bg.g, palette.bg.b, 0xff];
    for chunk in out.chunks_exact_mut(4) {
        if chunk[3] == 0 {
            chunk.copy_from_slice(&bg);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_status_bar(
    ui: &mut Ui,
    window_width: u32,
    window_height: u32,
    game: &Rect,
    rom: Option<&Rom>,
    settings: &Settings,
    base_cycles: u32,
    fps: u32,
    message: &str,
) {
    let free_bottom = window_height as i32 - (game.y() + game.height() as i32);
    if free_bottom < 26 {
        return;
    }

    let y = window_height as i32 - 24;
    let speed = match settings.speed {
        Speed::Custom => "custom".to_string(),
        other => other.label().to_string(),
    };
    let cycles = settings.effective_cycles(base_cycles);

    let left = match rom {
        Some(rom) => {
            let mut text = format!(
                "{} | {} | {cycles} cycles/frame ({speed}) | {fps} fps",
                rom.name, MODE_NAMES[settings.mode as usize]
            );
            if settings.paused {
                text.push_str(" | PAUSED");
            }
            if !message.is_empty() {
                text.push_str(" | ");
                text.push_str(message);
            }
            text
        }
        None => {
            if message.is_empty() {
                "No ROM loaded".to_string()
            } else {
                message.to_string()
            }
        }
    };

    ui.text(12, y, 2, TEXT, &left);

    let hint = "F1 settings | Space pause | F11 fullscreen | Esc quit";
    let hint_width = Ui::text_width(hint, 1);
    if window_width as i32 - 12 - hint_width > 12 + Ui::text_width(&left, 2) + 24 {
        ui.text(window_width as i32 - 12 - hint_width, y + 4, 1, MUTED, hint);
    }
}

#[derive(Default)]
struct MenuOutcome {
    close: bool,
    reset: bool,
    reload: bool,
}

#[allow(clippy::too_many_arguments)]
fn draw_menu(
    ui: &mut Ui,
    settings: &mut Settings,
    rom: Option<&Rom>,
    base_cycles: u32,
    outcome: &mut MenuOutcome,
) {
    let (window_width, window_height) = ui.window_size();
    ui.fill(Rect::new(0, 0, window_width, window_height), ui::DIMMER);

    let panel_width = 900.min(window_width.saturating_sub(40));
    let panel_height = 480;
    let panel = Rect::new(
        ((window_width - panel_width) / 2) as i32,
        ((window_height - panel_height) / 2) as i32,
        panel_width,
        panel_height,
    );
    ui.fill(panel, PANEL);
    ui.outline(panel, 2, PANEL_BORDER);

    let px = panel.x() + 30;
    let py = panel.y();
    let left = px;
    let right = px + 440;

    ui.text(px, py + 24, 3, TEXT, "SETTINGS");

    let close_rect = Rect::new(panel.x() + panel.width() as i32 - 66, py + 20, 40, 34);
    if ui.button(close_rect, "X", ui::Style::Normal) {
        outcome.close = true;
    }

    // ------------------------------------------------------------ modes ----
    ui.text(left, py + 72, 2, MUTED, "EMULATION");

    let mode_labels = ["VIP", "Octo", "SCHIP"];
    for (index, label) in mode_labels.iter().enumerate() {
        let rect = Rect::new(left + index as i32 * 126, py + 96, 118, 34);
        if ui.chip(rect, label, settings.mode as usize == index) {
            settings.mode = index as u8;
        }
    }

    // ------------------------------------------------------------ speed ----
    ui.text(left, py + 144, 2, MUTED, "SPEED");
    for (index, speed) in Speed::ALL.iter().enumerate() {
        let column = index % 4;
        let row = index / 4;
        let rect = Rect::new(
            left + column as i32 * 96,
            py + 166 + row as i32 * 36,
            90,
            30,
        );
        if ui.chip(rect, speed.label(), settings.speed == *speed) {
            settings.select_speed(*speed, base_cycles);
        }
    }

    // ----------------------------------------------------------- cycles ----
    let cycles = settings.effective_cycles(base_cycles);
    ui.text(left, py + 284, 2, MUTED, "CYCLES/FRAME");

    let minus = Rect::new(left, py + 306, 36, 34);
    if ui.button(minus, "-", ui::Style::Normal) {
        settings.nudge_cycles(-10, base_cycles);
    }
    let value_rect = Rect::new(left + 44, py + 306, 160, 34);
    ui.fill(value_rect, FIELD);
    ui.outline(value_rect, 1, ui::BORDER);
    let value_text = cycles.to_string();
    ui.centered_text(value_rect, 2, ACCENT, &value_text);

    let plus = Rect::new(left + 212, py + 306, 36, 34);
    if ui.button(plus, "+", ui::Style::Normal) {
        settings.nudge_cycles(10, base_cycles);
    }

    let hint = if settings.speed == Speed::Custom {
        "custom"
    } else {
        "from speed"
    };
    ui.text(left + 262, py + 315, 2, MUTED, hint);

    // ------------------------------------------------------ pause/display ----
    let pause = Rect::new(left, py + 348, 300, 32);
    let pause_label = if settings.paused {
        "PAUSE: ON"
    } else {
        "PAUSE: OFF"
    };
    if ui.toggle(pause, pause_label, settings.paused) {
        settings.paused = !settings.paused;
    }

    let full = Rect::new(left, py + 388, 300, 32);
    let full_label = if settings.fullscreen {
        "FULLSCREEN: ON"
    } else {
        "FULLSCREEN: OFF"
    };
    if ui.toggle(full, full_label, settings.fullscreen) {
        settings.fullscreen = !settings.fullscreen;
    }

    // ----------------------------------------------------------- colours ----
    ui.text(right, py + 72, 2, MUTED, "GRAPHICS");
    let preset_name = settings.preset().unwrap_or("CUSTOM");
    let preset_x = right + 400 - Ui::text_width(preset_name, 2);
    ui.text(preset_x, py + 72, 2, ACCENT, preset_name);

    for (index, channel) in Channel::ALL.iter().enumerate() {
        let rect = Rect::new(right + index as i32 * 100, py + 96, 92, 42);
        if ui.swatch(
            rect,
            channel.label(),
            channel.color(&settings.palette),
            settings.channel == *channel,
        ) {
            settings.channel = *channel;
        }
    }

    let current = settings.channel.color(&settings.palette);
    ui.text(right, py + 154, 2, MUTED, "R");
    let mut red = current.r;
    let mut green = current.g;
    let mut blue = current.b;

    let red_rect = Rect::new(right + 30, py + 158, 300, 18);
    let green_rect = Rect::new(right + 30, py + 198, 300, 18);
    let blue_rect = Rect::new(right + 30, py + 238, 300, 18);

    let mut changed = false;
    changed |= ui.slider(red_rect, &mut red, settings::Rgb::new(0xff, 0x40, 0x40));
    ui.text(right, py + 194, 2, MUTED, "G");
    changed |= ui.slider(green_rect, &mut green, settings::Rgb::new(0x40, 0xd0, 0x60));
    ui.text(right, py + 234, 2, MUTED, "B");
    changed |= ui.slider(blue_rect, &mut blue, settings::Rgb::new(0x50, 0x90, 0xff));

    ui.text(right + 348, py + 158, 2, TEXT, &red.to_string());
    ui.text(right + 348, py + 198, 2, TEXT, &green.to_string());
    ui.text(right + 348, py + 238, 2, TEXT, &blue.to_string());

    if changed {
        let target = settings.channel.color_mut(&mut settings.palette);
        target.r = red;
        target.g = green;
        target.b = blue;
    }

    // ----------------------------------------------------------- presets ----
    ui.text(right, py + 280, 2, MUTED, "PRESETS");
    for (index, preset) in PRESETS.iter().enumerate() {
        let column = (index % 3) as i32;
        let row = (index / 3) as i32;
        let rect = Rect::new(right + column * 130, py + 302 + row * 36, 124, 32);
        if ui.swatch(
            rect,
            preset.short,
            preset.palette.p1,
            settings.palette == preset.palette,
        ) {
            settings.palette = preset.palette;
        }
    }

    ui.text(
        right,
        py + 384,
        1,
        MUTED,
        "SPACE pause   F11 fullscreen   F1 menu",
    );
    ui.text(
        right,
        py + 394,
        1,
        MUTED,
        "DROP a ROM file onto the window to load it",
    );
    ui.text(right, py + 408, 1, MUTED, "KEYS 1234 QWER ASDF ZXCV");

    // ----------------------------------------------------------- buttons ----
    let footer_y = panel.y() + panel.height() as i32 - 56;
    let mut status = match rom {
        Some(rom) => format!("{} ({} bytes)", rom.name, rom.data.len()),
        None => "No ROM loaded - drop a file onto the window".to_string(),
    };
    if status.chars().count() > 28 {
        status = format!("{}...", status.chars().take(25).collect::<String>());
    }
    ui.text(panel.x() + 30, footer_y + 10, 2, MUTED, &status);

    let close = Rect::new(panel.x() + panel.width() as i32 - 136, footer_y, 106, 36);
    if ui.button(close, "CLOSE", ui::Style::Accent) {
        outcome.close = true;
    }
    let reload = Rect::new(panel.x() + panel.width() as i32 - 256, footer_y, 106, 36);
    if ui.button(reload, "RELOAD", ui::Style::Normal) && rom.is_some() {
        outcome.reload = true;
    }
    let reset = Rect::new(panel.x() + panel.width() as i32 - 376, footer_y, 106, 36);
    if ui.button(reset, "RESET", ui::Style::Normal) && rom.is_some() {
        outcome.reset = true;
    }
}

fn key2btn(key: Keycode) -> Option<usize> {
    match key {
        Keycode::Num1 => Some(0x1),
        Keycode::Num2 => Some(0x2),
        Keycode::Num3 => Some(0x3),
        Keycode::Num4 => Some(0xC),
        Keycode::Q => Some(0x4),
        Keycode::W => Some(0x5),
        Keycode::E => Some(0x6),
        Keycode::R => Some(0xD),
        Keycode::A => Some(0x7),
        Keycode::S => Some(0x8),
        Keycode::D => Some(0x9),
        Keycode::F => Some(0xE),
        Keycode::Z => Some(0xA),
        Keycode::X => Some(0x0),
        Keycode::C => Some(0xB),
        Keycode::V => Some(0xF),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_RATE: f32 = 48_000.0;

    fn render(pattern: [u8; 16], pitch: u8, seconds: f32) -> Vec<f32> {
        let shared = Arc::new(Mutex::new(AudioState {
            pattern,
            pitch,
            playing: true,
        }));
        let mut buzzer = XoBuzzer::new(shared, TEST_RATE);
        let mut out = vec![0.0; (TEST_RATE * seconds) as usize];
        AudioCallback::callback(&mut buzzer, &mut out);
        out
    }

    fn peak(samples: &[f32]) -> f32 {
        let skip = (TEST_RATE * 0.2) as usize;
        samples[skip..]
            .iter()
            .fold(0.0f32, |max, sample| max.max(sample.abs()))
    }

    fn zero_crossings_hz(samples: &[f32]) -> f32 {
        let skip = (TEST_RATE * 0.2) as usize;
        let mut crossings = 0;
        for pair in samples[skip..].windows(2) {
            if (pair[0] < 0.0 && pair[1] >= 0.0) || (pair[0] >= 0.0 && pair[1] < 0.0) {
                crossings += 1;
            }
        }
        (crossings as f32 * TEST_RATE) / (2.0 * (samples.len() - skip) as f32)
    }

    #[test]
    fn zero_pattern_is_silent() {
        let samples = render([0; 16], 64, 1.0);
        assert!(peak(&samples) < 0.001);
    }

    #[test]
    fn default_pattern_is_about_500_hz() {
        let samples = render([0x0F; 16], 64, 1.0);
        assert!(peak(&samples) > 0.02);
        let hz = zero_crossings_hz(&samples);
        assert!((hz - 500.0).abs() < 25.0, "measured {hz} Hz");
    }

    #[test]
    fn spec_a4_pattern_is_about_439_hz() {
        let mut pattern = [0u8; 16];
        pattern[8..].fill(0xFF);
        let samples = render(pattern, 247, 1.0);
        let hz = zero_crossings_hz(&samples);
        assert!((hz - 439.0).abs() < 25.0, "measured {hz} Hz");
    }

    #[test]
    fn game_rect_is_integer_scaled_and_centred() {
        let rect = game_rect(1280, 720);
        assert_eq!(
            (rect.x(), rect.y(), rect.width(), rect.height()),
            (0, 40, 1280, 640)
        );

        let small = game_rect(800, 600);
        assert_eq!((small.width(), small.height()), (768, 384));
        assert_eq!((small.x(), small.y()), (16, 108));
    }

    #[test]
    fn pixels_use_the_selected_palette() {
        let mut emu = Emu::default();
        // I = 0x20A, V0 = V1 = 0, draw one 8-pixel row, then loop forever.
        let rom = [
            0xA2, 0x0A, 0x60, 0x00, 0x61, 0x00, 0xD0, 0x11, 0x12, 0x06, 0xFF,
        ];
        assert!(emu.load(&rom));
        for _ in 0..4 {
            emu.tick();
        }

        let palette = Palette::new(
            settings::Rgb::new(0x11, 0x22, 0x33),
            settings::Rgb::new(0, 0, 0),
            settings::Rgb::new(0, 0, 0),
            settings::Rgb::new(0xAA, 0xBB, 0xCC),
        );
        let mut pixels = vec![0u8; SCREEN_WIDTH * SCREEN_HEIGHT * 4];
        fill_pixels(&emu, &palette, &mut pixels);

        assert_eq!(&pixels[0..4], &[0x11, 0x22, 0x33, 0xff]);
        // VIP mode doubles the sprite, so row 0 and row 1 are lit.
        let background = SCREEN_WIDTH * 4 * 2;
        assert_eq!(
            &pixels[background..background + 4],
            &[0xAA, 0xBB, 0xCC, 0xff]
        );
    }
}
