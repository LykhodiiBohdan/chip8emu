use chip8_core::*;
use js_sys::{Uint8Array, Uint8ClampedArray};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, ImageData, KeyboardEvent};

const PIXEL_COUNT: usize = SCREEN_WIDTH * SCREEN_HEIGHT;

#[wasm_bindgen]
pub struct EmuWasm {
    chip8: Emu,
    ctx: CanvasRenderingContext2d,
    /// 128x64 scratch canvas that receives `image` before it is scaled up in
    /// one `drawImage` call. Drawing thousands of `fillRect`s per frame was
    /// the main cost on low-end phones.
    buffer: HtmlCanvasElement,
    buffer_ctx: CanvasRenderingContext2d,
    image: ImageData,
    rgba: Uint8ClampedArray,
    pixels: Vec<u8>,
}

#[wasm_bindgen]
impl EmuWasm {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Result<EmuWasm, JsValue> {
        let chip8 = Emu::default();

        let document = web_sys::window().unwrap().document().unwrap();
        let canvas = document.get_element_by_id("canvas").unwrap();
        let canvas: HtmlCanvasElement = canvas
            .dyn_into::<HtmlCanvasElement>()
            .map_err(|_| ())
            .unwrap();

        let ctx = canvas
            .get_context("2d")
            .unwrap()
            .unwrap()
            .dyn_into::<CanvasRenderingContext2d>()
            .unwrap();
        // The 128x64 buffer is scaled by an integer factor; smoothing would
        // blur the pixels.
        ctx.set_image_smoothing_enabled(false);

        let buffer: HtmlCanvasElement = document
            .create_element("canvas")
            .unwrap()
            .dyn_into::<HtmlCanvasElement>()
            .unwrap();
        buffer.set_width(SCREEN_WIDTH as u32);
        buffer.set_height(SCREEN_HEIGHT as u32);

        let buffer_ctx = buffer
            .get_context("2d")
            .unwrap()
            .unwrap()
            .dyn_into::<CanvasRenderingContext2d>()
            .unwrap();

        let image = buffer_ctx
            .create_image_data_with_sw_and_sh(SCREEN_WIDTH as f64, SCREEN_HEIGHT as f64)
            .unwrap();
        // `ImageData::data()` in web-sys copies, so grab the live backing
        // `Uint8ClampedArray` instead and write into it in place.
        let rgba: Uint8ClampedArray = js_sys::Reflect::get(&image, &JsValue::from_str("data"))
            .unwrap()
            .dyn_into()
            .unwrap();

        Ok(EmuWasm {
            chip8,
            ctx,
            buffer,
            buffer_ctx,
            image,
            rgba,
            pixels: vec![0; PIXEL_COUNT * 4],
        })
    }

    #[wasm_bindgen]
    pub fn tick(&mut self) {
        self.chip8.tick();
    }

    #[wasm_bindgen]
    pub fn tick_timers(&mut self) {
        self.chip8.tick_timers();
    }

    #[wasm_bindgen]
    pub fn take_vblank(&mut self) -> bool {
        self.chip8.take_vblank()
    }

    #[wasm_bindgen]
    pub fn reset(&mut self) {
        self.chip8.reset();
    }

    #[wasm_bindgen]
    pub fn keypress(&mut self, evt: KeyboardEvent, pressed: bool) {
        let key = evt.key().to_lowercase();
        if let Some(k) = key2btn(&key) {
            self.chip8.keypress(k, pressed);
        }
    }

    #[wasm_bindgen]
    pub fn load_game(&mut self, data: Uint8Array) -> bool {
        self.chip8.load(&data.to_vec())
    }

    #[wasm_bindgen]
    pub fn is_beeping(&self) -> bool {
        self.chip8.is_beeping()
    }

    /// The 16-byte XO-CHIP audio pattern buffer.
    #[wasm_bindgen]
    pub fn audio_pattern(&self) -> Vec<u8> {
        self.chip8.audio_pattern().to_vec()
    }

    /// XO-CHIP playback pitch. The played bit rate is
    /// `4000 * 2^((pitch - 64) / 48)` Hz.
    #[wasm_bindgen]
    pub fn pitch(&self) -> u8 {
        self.chip8.pitch()
    }

    #[wasm_bindgen]
    pub fn is_schip(&self) -> bool {
        self.chip8.is_schip()
    }

    #[wasm_bindgen]
    pub fn is_halted(&self) -> bool {
        self.chip8.is_halted()
    }

    #[wasm_bindgen]
    pub fn halt_reason(&self) -> String {
        match self.chip8.halt_reason() {
            None => String::new(),
            Some(HaltReason::Exit) => "Program exited (00FD)".to_string(),
            Some(HaltReason::InvalidOpcode(op)) => format!("Invalid opcode {op:04X}"),
            Some(HaltReason::StackOverflow) => "Call stack overflow".to_string(),
            Some(HaltReason::StackUnderflow) => "Call stack underflow".to_string(),
        }
    }

    #[wasm_bindgen]
    pub fn set_mode(&mut self, mode_id: u8) {
        self.chip8.set_mode(mode_id);
    }

    #[wasm_bindgen]
    pub fn get_mode(&self) -> u8 {
        self.chip8.get_mode()
    }

    #[wasm_bindgen]
    pub fn cycles_per_frame(&self) -> u32 {
        self.chip8.cycles_per_frame()
    }

    #[wasm_bindgen]
    pub fn is_hi_res(&self) -> bool {
        self.chip8.is_hi_res()
    }

    /// Render both XO-CHIP planes. `c1` is plane 1, `c2` is plane 2 and
    /// `c3` is the overlap colour. Colours are 0xRRGGBB.
    #[wasm_bindgen]
    pub fn draw_screen(&mut self, scale: usize, c1: u32, c2: u32, c3: u32) {
        let colors = [rgba(c1), rgba(c2), rgba(c3)];

        // Clearing to transparent keeps the background colour that the caller
        // painted before this call, and lets one scaled `drawImage` do all the
        // work instead of thousands of `fillRect`s.
        self.pixels.fill(0);
        {
            let plane0 = self.chip8.get_plane(0);
            let plane1 = self.chip8.get_plane(1);
            for (idx, (low, high)) in plane0.iter().zip(plane1).enumerate() {
                let pixel = *low as u8 | ((*high as u8) << 1);
                if pixel == 0 {
                    continue;
                }
                let offset = idx * 4;
                self.pixels[offset..offset + 4].copy_from_slice(&colors[pixel as usize - 1]);
            }
        }

        self.rgba.copy_from(&self.pixels);
        let _ = self.buffer_ctx.put_image_data(&self.image, 0.0, 0.0);

        let width = (SCREEN_WIDTH * scale) as f64;
        let height = (SCREEN_HEIGHT * scale) as f64;
        let _ = self.ctx.draw_image_with_html_canvas_element_and_dw_and_dh(
            &self.buffer,
            0.0,
            0.0,
            width,
            height,
        );
    }
}

fn rgba(color: u32) -> [u8; 4] {
    [(color >> 16) as u8, (color >> 8) as u8, color as u8, 0xFF]
}

fn key2btn(key: &str) -> Option<usize> {
    match key {
        "1" => Some(0x1),
        "2" => Some(0x2),
        "3" => Some(0x3),
        "4" => Some(0xC),
        "q" => Some(0x4),
        "w" => Some(0x5),
        "e" => Some(0x6),
        "r" => Some(0xD),
        "a" => Some(0x7),
        "s" => Some(0x8),
        "d" => Some(0x9),
        "f" => Some(0xE),
        "z" => Some(0xA),
        "x" => Some(0x0),
        "c" => Some(0xB),
        "v" => Some(0xF),
        _ => None,
    }
}
