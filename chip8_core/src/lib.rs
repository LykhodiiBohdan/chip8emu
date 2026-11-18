use rand::random;

pub const SCREEN_WIDTH: usize = 128;
pub const SCREEN_HEIGHT: usize = 64;

const PIXELS: usize = SCREEN_WIDTH * SCREEN_HEIGHT;

// XO-CHIP address space. Non-XO profiles only use the low 4K.
const RAM_SIZE: usize = 0x10000;
const NUM_REGS: usize = 16;
const STACK_SIZE: usize = 16;
const NUM_KEYS: usize = 16;
const START_ADDR: u16 = 0x200;

/// Largest ROM that fits after the interpreter's font area.
pub const MAX_ROM_SIZE: usize = RAM_SIZE - START_ADDR as usize;

/// Default XO-CHIP audio pattern: four samples low and four high, i.e. a
/// 500 Hz buzzer at the default pitch of 64. XO-CHIP programs normally load
/// their own pattern with `F002`; classic games rely on this default so the
/// buzzer keeps working without any audio instructions.
pub const DEFAULT_AUDIO_PATTERN: [u8; 16] = [0x0F; 16];

const FONTSET_SIZE: usize = 80;
const FONTSET: [u8; FONTSET_SIZE] = [
    0xF0, 0x90, 0x90, 0x90, 0xF0, // 0
    0x20, 0x60, 0x20, 0x20, 0x70, // 1
    0xF0, 0x10, 0xF0, 0x80, 0xF0, // 2
    0xF0, 0x10, 0xF0, 0x10, 0xF0, // 3
    0x90, 0x90, 0xF0, 0x10, 0x10, // 4
    0xF0, 0x80, 0xF0, 0x10, 0xF0, // 5
    0xF0, 0x80, 0xF0, 0x90, 0xF0, // 6
    0xF0, 0x10, 0x20, 0x40, 0x40, // 7
    0xF0, 0x90, 0xF0, 0x90, 0xF0, // 8
    0xF0, 0x90, 0xF0, 0x10, 0xF0, // 9
    0xF0, 0x90, 0xF0, 0x90, 0x90, // A
    0xE0, 0x90, 0xE0, 0x90, 0xE0, // B
    0xF0, 0x80, 0x80, 0x80, 0xF0, // C
    0xE0, 0x90, 0x90, 0x90, 0xE0, // D
    0xF0, 0x80, 0xF0, 0x80, 0xF0, // E
    0xF0, 0x80, 0xF0, 0x80, 0x80, // F
];

const BIG_FONTSET_SIZE: usize = 160;
const BIG_FONTSET: [u8; BIG_FONTSET_SIZE] = [
    0x3C, 0x7E, 0xE7, 0xC3, 0xC3, 0xC3, 0xC3, 0xE7, 0x7E, 0x3C, // 0
    0x18, 0x38, 0x58, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x3C, // 1
    0x3E, 0x7F, 0xC3, 0x06, 0x0C, 0x18, 0x30, 0x60, 0xFF, 0xFF, // 2
    0x3C, 0x7E, 0xC3, 0x03, 0x0E, 0x0E, 0x03, 0xC3, 0x7E, 0x3C, // 3
    0x06, 0x0E, 0x1E, 0x36, 0x66, 0xC6, 0xFF, 0xFF, 0x06, 0x06, // 4
    0xFF, 0xFF, 0xC0, 0xC0, 0xFC, 0xFE, 0x03, 0xC3, 0x7E, 0x3C, // 5
    0x3E, 0x7C, 0xC0, 0xC0, 0xFC, 0xFE, 0xC3, 0xC3, 0x7E, 0x3C, // 6
    0xFF, 0xFF, 0x03, 0x06, 0x0C, 0x18, 0x30, 0x60, 0x60, 0x60, // 7
    0x3C, 0x7E, 0xC3, 0xC3, 0x7E, 0x7E, 0xC3, 0xC3, 0x7E, 0x3C, // 8
    0x3C, 0x7E, 0xC3, 0xC3, 0x7F, 0x3F, 0x03, 0x03, 0x3E, 0x7C, // 9
    0x3C, 0x7E, 0xC3, 0xC3, 0xFF, 0xFF, 0xC3, 0xC3, 0xC3, 0xC3, // A
    0xFC, 0xFE, 0xC3, 0xC3, 0xFC, 0xFC, 0xC3, 0xC3, 0xFE, 0xFC, // B
    0x3E, 0x7F, 0xC0, 0xC0, 0xC0, 0xC0, 0xC0, 0xC0, 0x7F, 0x3E, // C
    0xFC, 0xFE, 0xC3, 0xC3, 0xC3, 0xC3, 0xC3, 0xC3, 0xFE, 0xFC, // D
    0xFF, 0xFF, 0xC0, 0xC0, 0xFC, 0xFC, 0xC0, 0xC0, 0xFF, 0xFF, // E
    0xFF, 0xFF, 0xC0, 0xC0, 0xFC, 0xFC, 0xC0, 0xC0, 0xC0, 0xC0, // F
];

/// Per-ROM compatibility switches.
///
/// These mirror Octo's quirk options. The important part is that they are
/// independent: resolution (`hires`) is not a "mode", and a game may mix
/// hires with either quirk profile.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Quirks {
    /// `8XY6`/`8XYE` shift VX instead of VY.
    pub shift_uses_vx: bool,
    /// `FX55`/`FX65` leave I unchanged.
    pub load_store_keeps_i: bool,
    /// `8XY1`/`8XY2`/`8XY3` reset VF to zero.
    pub logic_resets_vf: bool,
    /// Sprites are clipped at the screen edge instead of wrapping.
    pub clip_sprites: bool,
    /// `BNNN` adds VX (index taken from NNN) instead of V0.
    pub jump_uses_vx: bool,
    /// Drawing waits for the next vblank.
    pub vblank: bool,
    /// Enable the 64K address space and XO-CHIP instructions.
    pub xo_chip: bool,
}

impl Quirks {
    /// Original COSMAC VIP / CHIP-8 behaviour.
    pub const VIP: Self = Self {
        shift_uses_vx: false,
        load_store_keeps_i: false,
        logic_resets_vf: true,
        clip_sprites: true,
        jump_uses_vx: false,
        vblank: true,
        xo_chip: false,
    };

    /// Super-CHIP 1.1 behaviour.
    pub const SCHIP: Self = Self {
        shift_uses_vx: true,
        load_store_keeps_i: true,
        logic_resets_vf: false,
        clip_sprites: true,
        jump_uses_vx: true,
        vblank: false,
        xo_chip: false,
    };

    /// Octo defaults. This is also the XO-CHIP profile.
    pub const OCTO: Self = Self {
        shift_uses_vx: false,
        load_store_keeps_i: false,
        logic_resets_vf: false,
        clip_sprites: false,
        jump_uses_vx: false,
        vblank: false,
        xo_chip: true,
    };
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EmuMode {
    Chip8,
    Octo,
    Schip,
}

impl EmuMode {
    pub fn quirks(self) -> Quirks {
        match self {
            EmuMode::Chip8 => Quirks::VIP,
            EmuMode::Octo => Quirks::OCTO,
            EmuMode::Schip => Quirks::SCHIP,
        }
    }

    /// Cycles per 60 Hz frame. Octo's default tickrate is 20.
    pub fn cycles_per_frame(self) -> u32 {
        match self {
            EmuMode::Chip8 => 15,
            EmuMode::Octo => 20,
            EmuMode::Schip => 30,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            EmuMode::Chip8 => "CHIP-8 (VIP)",
            EmuMode::Octo => "Octo / XO-CHIP",
            EmuMode::Schip => "Super-CHIP",
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum HaltReason {
    Exit,
    InvalidOpcode(u16),
    StackOverflow,
    StackUnderflow,
}

pub struct Emu {
    pc: u16,
    ram: Box<[u8; RAM_SIZE]>,
    /// Two XO-CHIP drawing planes, stored at 128x64. In lores mode each
    /// logical pixel occupies a 2x2 block.
    screen: Box<[[bool; PIXELS]; 2]>,
    v_reg: [u8; NUM_REGS],
    i_reg: u16,
    sp: usize,
    stack: [u16; STACK_SIZE],
    keys: [bool; NUM_KEYS],
    dt: u8,
    st: u8,
    hi_res: bool,
    halted: bool,
    halt_reason: Option<HaltReason>,
    last_op: u16,
    rpl_flags: [u8; NUM_REGS],
    mode: EmuMode,
    wait_key: Option<usize>,
    plane: u8,
    pitch: u8,
    pattern: [u8; 16],
    vblank_hit: bool,
}

impl Default for Emu {
    fn default() -> Self {
        Self::new()
    }
}

impl Emu {
    pub fn new() -> Self {
        let mut new_emu = Self {
            pc: START_ADDR,
            ram: Box::new([0; RAM_SIZE]),
            screen: Box::new([[false; PIXELS]; 2]),
            v_reg: [0; NUM_REGS],
            i_reg: 0,
            sp: 0,
            stack: [0; STACK_SIZE],
            keys: [false; NUM_KEYS],
            dt: 0,
            st: 0,
            hi_res: false,
            halted: false,
            halt_reason: None,
            last_op: 0,
            rpl_flags: [0; NUM_REGS],
            mode: EmuMode::Octo,
            wait_key: None,
            plane: 1,
            pitch: 64,
            pattern: DEFAULT_AUDIO_PATTERN,
            vblank_hit: false,
        };
        new_emu.load_fonts();
        new_emu
    }

    /// Reset the machine but keep the selected compatibility profile.
    pub fn reset(&mut self) {
        self.pc = START_ADDR;
        self.ram.fill(0);
        for layer in self.screen.iter_mut() {
            layer.fill(false);
        }
        self.v_reg = [0; NUM_REGS];
        self.i_reg = 0;
        self.sp = 0;
        self.stack = [0; STACK_SIZE];
        self.keys = [false; NUM_KEYS];
        self.dt = 0;
        self.st = 0;
        self.hi_res = false;
        self.halted = false;
        self.halt_reason = None;
        self.last_op = 0;
        self.rpl_flags = [0; NUM_REGS];
        self.wait_key = None;
        self.plane = 1;
        self.pitch = 64;
        self.pattern = DEFAULT_AUDIO_PATTERN;
        self.vblank_hit = false;
        self.load_fonts();
    }

    /// Load a ROM at 0x200. The selected profile is preserved.
    pub fn load(&mut self, data: &[u8]) -> bool {
        let max_size = MAX_ROM_SIZE;
        if data.is_empty() || data.len() > max_size {
            return false;
        }
        self.reset();
        let start = START_ADDR as usize;
        self.ram[start..start + data.len()].copy_from_slice(data);
        true
    }

    pub fn set_mode(&mut self, mode_id: u8) {
        self.mode = match mode_id {
            0 => EmuMode::Chip8,
            2 => EmuMode::Schip,
            _ => EmuMode::Octo,
        };
    }

    pub fn get_mode(&self) -> u8 {
        match self.mode {
            EmuMode::Chip8 => 0,
            EmuMode::Octo => 1,
            EmuMode::Schip => 2,
        }
    }

    pub fn is_schip(&self) -> bool {
        self.mode == EmuMode::Schip
    }

    pub fn quirks(&self) -> Quirks {
        self.mode.quirks()
    }

    pub fn cycles_per_frame(&self) -> u32 {
        self.mode.cycles_per_frame()
    }

    pub fn mode_name(&self) -> &'static str {
        self.mode.name()
    }

    pub fn is_halted(&self) -> bool {
        self.halted
    }

    pub fn halt_reason(&self) -> Option<HaltReason> {
        self.halt_reason
    }

    pub fn last_opcode(&self) -> u16 {
        self.last_op
    }

    pub fn is_beeping(&self) -> bool {
        self.st > 0
    }

    pub fn is_hi_res(&self) -> bool {
        self.hi_res
    }

    pub fn plane(&self) -> u8 {
        self.plane
    }

    pub fn pitch(&self) -> u8 {
        self.pitch
    }

    pub fn audio_pattern(&self) -> &[u8; 16] {
        &self.pattern
    }

    pub fn get_reg(&self, idx: usize) -> u8 {
        self.v_reg[idx & 0xF]
    }

    pub fn get_i(&self) -> u16 {
        self.i_reg
    }

    pub fn get_pc(&self) -> u16 {
        self.pc
    }

    pub fn get_sp(&self) -> usize {
        self.sp
    }

    pub fn get_ram(&self, addr: usize) -> u8 {
        self.ram[addr & 0xFFFF]
    }

    pub fn get_display(&self) -> &[bool] {
        &self.screen[0]
    }

    pub fn get_plane(&self, layer: usize) -> &[bool] {
        &self.screen[layer & 1]
    }

    /// Combined XO-CHIP pixel value: bit 0 = plane 1, bit 1 = plane 2.
    pub fn get_pixel(&self, x: usize, y: usize) -> u8 {
        if x >= SCREEN_WIDTH || y >= SCREEN_HEIGHT {
            return 0;
        }
        let idx = x + SCREEN_WIDTH * y;
        (self.screen[0][idx] as u8) | ((self.screen[1][idx] as u8) << 1)
    }

    pub fn keypress(&mut self, idx: usize, pressed: bool) {
        if idx < NUM_KEYS {
            self.keys[idx] = pressed;
        }
    }

    pub fn tick_timers(&mut self) {
        self.vblank_hit = false;
        if self.dt > 0 {
            self.dt -= 1;
        }
        if self.st > 0 {
            self.st -= 1;
        }
    }

    /// True if the current frame executed a drawing instruction in a profile
    /// with `vblank` enabled. Hosts should stop the rest of the frame's
    /// cycles when this returns true.
    pub fn take_vblank(&mut self) -> bool {
        let hit = self.vblank_hit;
        self.vblank_hit = false;
        hit
    }

    fn load_fonts(&mut self) {
        self.ram[..FONTSET_SIZE].copy_from_slice(&FONTSET);
        self.ram[FONTSET_SIZE..FONTSET_SIZE + BIG_FONTSET_SIZE].copy_from_slice(&BIG_FONTSET);
    }

    fn xo(&self) -> bool {
        self.mode.quirks().xo_chip
    }

    fn mem_mask(&self) -> usize {
        if self.xo() { 0xFFFF } else { 0x0FFF }
    }

    fn halt(&mut self, reason: HaltReason) {
        self.halted = true;
        if self.halt_reason.is_none() {
            self.halt_reason = Some(reason);
        }
    }

    fn push(&mut self, val: u16) {
        if self.sp >= STACK_SIZE {
            self.halt(HaltReason::StackOverflow);
            return;
        }
        self.stack[self.sp] = val;
        self.sp += 1;
    }

    fn pop(&mut self) -> Option<u16> {
        if self.sp == 0 {
            self.halt(HaltReason::StackUnderflow);
            return None;
        }
        self.sp -= 1;
        Some(self.stack[self.sp])
    }

    fn ram_at(&self, addr: usize) -> u8 {
        self.ram[addr & self.mem_mask()]
    }

    fn ram_write(&mut self, addr: usize, val: u8) {
        let mask = self.mem_mask();
        self.ram[addr & mask] = val;
    }

    pub fn tick(&mut self) {
        if self.halted {
            return;
        }
        let op = self.fetch();
        self.last_op = op;
        self.execute(op);
    }

    fn fetch(&mut self) -> u16 {
        let mask = self.mem_mask();
        let pc = (self.pc as usize) & mask;
        let higher_byte = self.ram[pc] as u16;
        let lower_byte = self.ram[(pc + 1) & mask] as u16;
        self.pc = ((pc + 2) & mask) as u16;
        (higher_byte << 8) | lower_byte
    }

    /// Skip the next instruction. In XO-CHIP a `F000 NNNN` pair is a single
    /// four-byte instruction and must be skipped as a whole.
    fn skip(&mut self) {
        let mask = self.mem_mask();
        let addr = (self.pc as usize) & mask;
        let next = ((self.ram[addr] as u16) << 8) | self.ram[(addr + 1) & mask] as u16;
        let step = if self.xo() && next == 0xF000 { 4 } else { 2 };
        self.pc = ((self.pc as usize + step) & mask) as u16;
    }

    fn draw_pixel(&mut self, x: usize, y: usize, layer: usize) -> bool {
        let (max_w, max_h) = if self.hi_res { (128, 64) } else { (64, 32) };

        if self.mode.quirks().clip_sprites && (x >= max_w || y >= max_h) {
            return false;
        }

        let (px, py) = (x % max_w, y % max_h);

        if self.hi_res {
            let idx = px + SCREEN_WIDTH * py;
            let collided = self.screen[layer][idx];
            self.screen[layer][idx] = !collided;
            collided
        } else {
            let sx = px * 2;
            let sy = py * 2;
            let idx = sx + SCREEN_WIDTH * sy;
            let collided = self.screen[layer][idx];
            let new_val = !collided;
            self.screen[layer][idx] = new_val;
            self.screen[layer][idx + 1] = new_val;
            self.screen[layer][idx + SCREEN_WIDTH] = new_val;
            self.screen[layer][idx + SCREEN_WIDTH + 1] = new_val;
            collided
        }
    }

    fn clear_selected_planes(&mut self) {
        for layer in 0..2 {
            if self.plane & (1 << layer) != 0 {
                self.screen[layer].fill(false);
            }
        }
    }

    fn scroll_down(&mut self, n: usize) {
        let dy = if self.hi_res { n } else { n * 2 };
        let dy = dy.min(SCREEN_HEIGHT);
        for layer in 0..2 {
            if self.plane & (1 << layer) == 0 {
                continue;
            }
            let screen = &mut self.screen[layer];
            for y in (dy..SCREEN_HEIGHT).rev() {
                for x in 0..SCREEN_WIDTH {
                    screen[x + SCREEN_WIDTH * y] = screen[x + SCREEN_WIDTH * (y - dy)];
                }
            }
            for y in 0..dy {
                for x in 0..SCREEN_WIDTH {
                    screen[x + SCREEN_WIDTH * y] = false;
                }
            }
        }
    }

    fn scroll_up(&mut self, n: usize) {
        let dy = if self.hi_res { n } else { n * 2 };
        let dy = dy.min(SCREEN_HEIGHT);
        for layer in 0..2 {
            if self.plane & (1 << layer) == 0 {
                continue;
            }
            let screen = &mut self.screen[layer];
            for y in 0..(SCREEN_HEIGHT - dy) {
                for x in 0..SCREEN_WIDTH {
                    screen[x + SCREEN_WIDTH * y] = screen[x + SCREEN_WIDTH * (y + dy)];
                }
            }
            for y in (SCREEN_HEIGHT - dy)..SCREEN_HEIGHT {
                for x in 0..SCREEN_WIDTH {
                    screen[x + SCREEN_WIDTH * y] = false;
                }
            }
        }
    }

    fn scroll_left(&mut self) {
        let shift = if self.hi_res { 4 } else { 8 };
        for layer in 0..2 {
            if self.plane & (1 << layer) == 0 {
                continue;
            }
            let screen = &mut self.screen[layer];
            for y in 0..SCREEN_HEIGHT {
                for x in 0..(SCREEN_WIDTH - shift) {
                    screen[x + SCREEN_WIDTH * y] = screen[(x + shift) + SCREEN_WIDTH * y];
                }
                for x in (SCREEN_WIDTH - shift)..SCREEN_WIDTH {
                    screen[x + SCREEN_WIDTH * y] = false;
                }
            }
        }
    }

    fn scroll_right(&mut self) {
        let shift = if self.hi_res { 4 } else { 8 };
        for layer in 0..2 {
            if self.plane & (1 << layer) == 0 {
                continue;
            }
            let screen = &mut self.screen[layer];
            for y in 0..SCREEN_HEIGHT {
                for x in (shift..SCREEN_WIDTH).rev() {
                    screen[x + SCREEN_WIDTH * y] = screen[(x - shift) + SCREEN_WIDTH * y];
                }
                for x in 0..shift {
                    screen[x + SCREEN_WIDTH * y] = false;
                }
            }
        }
    }

    fn execute(&mut self, op: u16) {
        let digit1 = (op & 0xF000) >> 12;
        let digit2 = (op & 0x0F00) >> 8;
        let digit3 = (op & 0x00F0) >> 4;
        let digit4 = op & 0x000F;
        let quirks = self.mode.quirks();
        let extended = self.mode != EmuMode::Chip8;

        match (digit1, digit2, digit3, digit4) {
            // NOP (0000). Other 0NNN machine-code calls are ignored below.
            (0, 0, 0, 0) => (),

            // Scroll Down N lines (00CN)
            (0, 0, 0xC, n) if extended => {
                self.scroll_down(n as usize);
            }

            // Scroll Up N lines (00DN, XO-CHIP)
            (0, 0, 0xD, n) if extended => {
                self.scroll_up(n as usize);
            }

            // CLS (00E0)
            (0, 0, 0xE, 0) => {
                self.clear_selected_planes();
            }

            // RET (00EE)
            (0, 0, 0xE, 0xE) => {
                if let Some(ret_addr) = self.pop() {
                    self.pc = ret_addr & self.mem_mask() as u16;
                }
            }

            // Scroll Right 4 pixels (00FB)
            (0, 0, 0xF, 0xB) if extended => {
                self.scroll_right();
            }

            // Scroll Left 4 pixels (00FC)
            (0, 0, 0xF, 0xC) if extended => {
                self.scroll_left();
            }

            // EXIT (00FD)
            (0, 0, 0xF, 0xD) if extended => {
                self.halt(HaltReason::Exit);
            }

            // Disable High-Res 64x32 (00FE), clears the display like Octo.
            (0, 0, 0xF, 0xE) if extended => {
                self.hi_res = false;
                for layer in self.screen.iter_mut() {
                    layer.fill(false);
                }
            }

            // Enable High-Res 128x64 (00FF), clears the display like Octo.
            (0, 0, 0xF, 0xF) if extended => {
                self.hi_res = true;
                for layer in self.screen.iter_mut() {
                    layer.fill(false);
                }
            }

            // JMP NNN (1NNN)
            (1, _, _, _) => {
                self.pc = (op & 0xFFF) & self.mem_mask() as u16;
            }

            // CALL NNN (2NNN)
            (2, _, _, _) => {
                let nnn = op & 0xFFF;
                self.push(self.pc);
                self.pc = nnn & self.mem_mask() as u16;
            }

            // SKIP VX == NN (3XNN)
            (3, _, _, _) => {
                let x = digit2 as usize;
                let nn = (op & 0xFF) as u8;
                if self.v_reg[x] == nn {
                    self.skip();
                }
            }

            // SKIP VX != NN (4XNN)
            (4, _, _, _) => {
                let x = digit2 as usize;
                let nn = (op & 0xFF) as u8;
                if self.v_reg[x] != nn {
                    self.skip();
                }
            }

            // SKIP VX == VY (5XY0)
            (5, _, _, 0) => {
                let x = digit2 as usize;
                let y = digit3 as usize;
                if self.v_reg[x] == self.v_reg[y] {
                    self.skip();
                }
            }

            // SAVE VX..VY (5XY2, XO-CHIP)
            (5, _, _, 2) if self.xo() => {
                let x = digit2 as usize;
                let y = digit3 as usize;
                let dist = x.abs_diff(y);
                let i = self.i_reg as usize;
                if x < y {
                    for z in 0..=dist {
                        let val = self.v_reg[x + z];
                        self.ram_write(i + z, val);
                    }
                } else {
                    for z in 0..=dist {
                        let val = self.v_reg[x - z];
                        self.ram_write(i + z, val);
                    }
                }
            }

            // LOAD VX..VY (5XY3, XO-CHIP)
            (5, _, _, 3) if self.xo() => {
                let x = digit2 as usize;
                let y = digit3 as usize;
                let dist = x.abs_diff(y);
                let i = self.i_reg as usize;
                if x < y {
                    for z in 0..=dist {
                        self.v_reg[x + z] = self.ram_at(i + z);
                    }
                } else {
                    for z in 0..=dist {
                        self.v_reg[x - z] = self.ram_at(i + z);
                    }
                }
            }

            // VX = NN (6XNN)
            (6, _, _, _) => {
                let x = digit2 as usize;
                self.v_reg[x] = (op & 0xFF) as u8;
            }

            // VX += NN (7XNN)
            (7, _, _, _) => {
                let x = digit2 as usize;
                let nn = (op & 0xFF) as u8;
                self.v_reg[x] = self.v_reg[x].wrapping_add(nn);
            }

            // VX = VY (8XY0)
            (8, _, _, 0) => {
                self.v_reg[digit2 as usize] = self.v_reg[digit3 as usize];
            }

            // VX |= VY (8XY1)
            (8, _, _, 1) => {
                let (x, y) = (digit2 as usize, digit3 as usize);
                self.v_reg[x] |= self.v_reg[y];
                if quirks.logic_resets_vf {
                    self.v_reg[0xF] = 0;
                }
            }

            // VX &= VY (8XY2)
            (8, _, _, 2) => {
                let (x, y) = (digit2 as usize, digit3 as usize);
                self.v_reg[x] &= self.v_reg[y];
                if quirks.logic_resets_vf {
                    self.v_reg[0xF] = 0;
                }
            }

            // VX ^= VY (8XY3)
            (8, _, _, 3) => {
                let (x, y) = (digit2 as usize, digit3 as usize);
                self.v_reg[x] ^= self.v_reg[y];
                if quirks.logic_resets_vf {
                    self.v_reg[0xF] = 0;
                }
            }

            // VX += VY (8XY4)
            (8, _, _, 4) => {
                let (x, y) = (digit2 as usize, digit3 as usize);
                let (new_vx, carry) = self.v_reg[x].overflowing_add(self.v_reg[y]);
                self.v_reg[x] = new_vx;
                self.v_reg[0xF] = carry as u8;
            }

            // VX -= VY (8XY5)
            (8, _, _, 5) => {
                let (x, y) = (digit2 as usize, digit3 as usize);
                let (new_vx, borrow) = self.v_reg[x].overflowing_sub(self.v_reg[y]);
                self.v_reg[x] = new_vx;
                self.v_reg[0xF] = (!borrow) as u8;
            }

            // VX >>= 1 (8XY6)
            (8, _, _, 6) => {
                let (x, y) = (digit2 as usize, digit3 as usize);
                let src = if quirks.shift_uses_vx { x } else { y };
                let val = self.v_reg[src];
                self.v_reg[x] = val >> 1;
                self.v_reg[0xF] = val & 1;
            }

            // VX = VY - VX (8XY7)
            (8, _, _, 7) => {
                let (x, y) = (digit2 as usize, digit3 as usize);
                let (new_vx, borrow) = self.v_reg[y].overflowing_sub(self.v_reg[x]);
                self.v_reg[x] = new_vx;
                self.v_reg[0xF] = (!borrow) as u8;
            }

            // VX <<= 1 (8XYE)
            (8, _, _, 0xE) => {
                let (x, y) = (digit2 as usize, digit3 as usize);
                let src = if quirks.shift_uses_vx { x } else { y };
                let val = self.v_reg[src];
                self.v_reg[x] = val << 1;
                self.v_reg[0xF] = (val >> 7) & 1;
            }

            // SKIP VX != VY (9XY0)
            (9, _, _, 0) => {
                let (x, y) = (digit2 as usize, digit3 as usize);
                if self.v_reg[x] != self.v_reg[y] {
                    self.skip();
                }
            }

            // I = NNN (ANNN)
            (0xA, _, _, _) => {
                self.i_reg = (op & 0xFFF) & self.mem_mask() as u16;
            }

            // JMP V0 + NNN (BNNN)
            (0xB, _, _, _) => {
                let nnn = op & 0xFFF;
                let offset_reg = if quirks.jump_uses_vx {
                    ((nnn >> 8) & 0xF) as usize
                } else {
                    0
                };
                let target = nnn.wrapping_add(self.v_reg[offset_reg] as u16);
                self.pc = target & self.mem_mask() as u16;
            }

            // VX = rand() & NN (CXNN)
            (0xC, _, _, _) => {
                let x = digit2 as usize;
                let nn = (op & 0xFF) as u8;
                let rng: u8 = random();
                self.v_reg[x] = rng & nn;
            }

            // DRAW (DXYN / DXY0)
            (0xD, _, _, _) => {
                let x_coord = self.v_reg[digit2 as usize] as usize;
                let y_coord = self.v_reg[digit3 as usize] as usize;
                let num_rows = digit4 as usize;
                let mut flipped = false;
                let mut data_addr = self.i_reg as usize;

                for layer in 0..2 {
                    if self.plane & (1 << layer) == 0 {
                        continue;
                    }

                    if num_rows == 0 {
                        for y_line in 0..16 {
                            let addr = data_addr + y_line * 2;
                            let row_bits =
                                u16::from_be_bytes([self.ram_at(addr), self.ram_at(addr + 1)]);
                            for x_line in 0..16 {
                                if (row_bits & (0x8000 >> x_line)) != 0
                                    && self.draw_pixel(x_coord + x_line, y_coord + y_line, layer)
                                {
                                    flipped = true;
                                }
                            }
                        }
                        data_addr += 32;
                    } else {
                        for y_line in 0..num_rows {
                            let pixels = self.ram_at(data_addr + y_line);
                            for x_line in 0..8 {
                                if (pixels & (0b1000_0000 >> x_line)) != 0
                                    && self.draw_pixel(x_coord + x_line, y_coord + y_line, layer)
                                {
                                    flipped = true;
                                }
                            }
                        }
                        data_addr += num_rows;
                    }
                }

                self.v_reg[0xF] = flipped as u8;

                if quirks.vblank {
                    self.vblank_hit = true;
                }
            }

            // SKIP KEY PRESS (EX9E)
            (0xE, _, 9, 0xE) => {
                let x = digit2 as usize;
                let vx = self.v_reg[x] as usize;
                if vx < NUM_KEYS && self.keys[vx] {
                    self.skip();
                }
            }

            // SKIP KEY RELEASE (EXA1)
            (0xE, _, 0xA, 1) => {
                let x = digit2 as usize;
                let vx = self.v_reg[x] as usize;
                if vx < NUM_KEYS && !self.keys[vx] {
                    self.skip();
                }
            }

            // I = long NNNN (F000, XO-CHIP)
            (0xF, 0, 0, 0) if self.xo() => {
                let hi = self.ram_at(self.pc as usize);
                let lo = self.ram_at(self.pc as usize + 1);
                self.i_reg = u16::from_be_bytes([hi, lo]);
                self.pc = self.pc.wrapping_add(2) & self.mem_mask() as u16;
            }

            // PLANE N (Fx01, XO-CHIP)
            (0xF, _, 0, 1) if self.xo() => {
                self.plane = (digit2 & 0x3) as u8;
            }

            // AUDIO pattern (F002, XO-CHIP)
            (0xF, _, 0, 2) if self.xo() => {
                let i = self.i_reg as usize;
                for z in 0..16 {
                    self.pattern[z] = self.ram_at(i + z);
                }
            }

            // VX = DT (FX07)
            (0xF, _, 0, 7) => {
                self.v_reg[digit2 as usize] = self.dt;
            }

            // WAIT KEY (FX0A): waits for press AND release, like Octo.
            (0xF, _, 0, 0xA) => {
                let x = digit2 as usize;
                if let Some(k) = self.wait_key {
                    if !self.keys[k] {
                        self.v_reg[x] = k as u8;
                        self.wait_key = None;
                        return;
                    }
                } else if let Some(k) = self.keys.iter().position(|&p| p) {
                    self.wait_key = Some(k);
                }
                self.pc = self.pc.wrapping_sub(2) & self.mem_mask() as u16;
            }

            // DT = VX (FX15)
            (0xF, _, 1, 5) => {
                self.dt = self.v_reg[digit2 as usize];
            }

            // ST = VX (FX18)
            (0xF, _, 1, 8) => {
                self.st = self.v_reg[digit2 as usize];
            }

            // I += VX (FX1E)
            (0xF, _, 1, 0xE) => {
                let vx = self.v_reg[digit2 as usize] as u16;
                self.i_reg = self.i_reg.wrapping_add(vx) & self.mem_mask() as u16;
            }

            // I = FONT (FX29)
            (0xF, _, 2, 9) => {
                let c = (self.v_reg[digit2 as usize] & 0xF) as u16;
                self.i_reg = c * 5;
            }

            // I = BIG FONT (FX30)
            (0xF, _, 3, 0) => {
                let c = (self.v_reg[digit2 as usize] & 0xF) as u16;
                self.i_reg = (FONTSET_SIZE as u16) + c * 10;
            }

            // BCD (FX33)
            (0xF, _, 3, 3) => {
                let vx = self.v_reg[digit2 as usize];
                let i = self.i_reg as usize;
                self.ram_write(i, vx / 100);
                self.ram_write(i + 1, (vx / 10) % 10);
                self.ram_write(i + 2, vx % 10);
            }

            // PITCH = VX (FX3A, XO-CHIP)
            (0xF, _, 3, 0xA) if self.xo() => {
                self.pitch = self.v_reg[digit2 as usize];
            }

            // STORE V0 - VX (FX55)
            (0xF, _, 5, 5) => {
                let x = digit2 as usize;
                let i = self.i_reg as usize;
                for idx in 0..=x {
                    let val = self.v_reg[idx];
                    self.ram_write(i + idx, val);
                }
                if !quirks.load_store_keeps_i {
                    self.i_reg = self.i_reg.wrapping_add(x as u16 + 1) & self.mem_mask() as u16;
                }
            }

            // LOAD V0 - VX (FX65)
            (0xF, _, 6, 5) => {
                let x = digit2 as usize;
                let i = self.i_reg as usize;
                for idx in 0..=x {
                    self.v_reg[idx] = self.ram_at(i + idx);
                }
                if !quirks.load_store_keeps_i {
                    self.i_reg = self.i_reg.wrapping_add(x as u16 + 1) & self.mem_mask() as u16;
                }
            }

            // STORE RPL FLAGS (FX75)
            (0xF, _, 7, 5) => {
                let max = if self.xo() { 0xF } else { 0x7 };
                let x = (digit2 as usize).min(max);
                for idx in 0..=x {
                    self.rpl_flags[idx] = self.v_reg[idx];
                }
            }

            // LOAD RPL FLAGS (FX85)
            (0xF, _, 8, 5) => {
                let max = if self.xo() { 0xF } else { 0x7 };
                let x = (digit2 as usize).min(max);
                for idx in 0..=x {
                    self.v_reg[idx] = self.rpl_flags[idx];
                }
            }

            // Machine code calls and unsupported syscalls are ignored.
            (0, _, _, _) => (),

            (_, _, _, _) => {
                self.halt(HaltReason::InvalidOpcode(op));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emu_with(rom: &[u8], mode: u8) -> Emu {
        let mut emu = Emu::new();
        emu.set_mode(mode);
        assert!(emu.load(rom));
        emu
    }

    #[test]
    fn big_font_has_letters() {
        for ch in 0xA..=0xF {
            let glyph = &BIG_FONTSET[ch * 10..ch * 10 + 10];
            assert!(glyph.iter().any(|&b| b != 0), "glyph {ch:X} is empty");
        }
    }

    #[test]
    fn shift_quirk_is_profile_specific() {
        // vA = vB >> 1, vB = 4
        let rom = [0x6A, 0x02, 0x6B, 0x04, 0x8A, 0xB6];

        let mut vip = emu_with(&rom, 0);
        for _ in 0..3 {
            vip.tick();
        }
        assert_eq!(vip.get_reg(0xA), 2);
        assert_eq!(vip.get_reg(0xF), 0);

        let mut schip = emu_with(&rom, 2);
        for _ in 0..3 {
            schip.tick();
        }
        assert_eq!(schip.get_reg(0xA), 1);
        assert_eq!(schip.get_reg(0xF), 0);

        let mut octo = emu_with(&rom, 1);
        for _ in 0..3 {
            octo.tick();
        }
        assert_eq!(octo.get_reg(0xA), 2);
    }

    #[test]
    fn logic_quirk_resets_vf_only_for_vip() {
        // vF = 5; vA = 1; vB = 2; vA |= vB
        let rom = [0x6F, 0x05, 0x6A, 0x01, 0x6B, 0x02, 0x8A, 0xB1];

        let mut vip = emu_with(&rom, 0);
        for _ in 0..4 {
            vip.tick();
        }
        assert_eq!(vip.get_reg(0xA), 3);
        assert_eq!(vip.get_reg(0xF), 0);

        let mut octo = emu_with(&rom, 1);
        for _ in 0..4 {
            octo.tick();
        }
        assert_eq!(octo.get_reg(0xA), 3);
        assert_eq!(octo.get_reg(0xF), 5);

        let mut schip = emu_with(&rom, 2);
        for _ in 0..4 {
            schip.tick();
        }
        assert_eq!(schip.get_reg(0xF), 5);
    }

    #[test]
    fn clip_vs_wrap() {
        // i = 0x300; vA = 62; vB = 0; draw 8x1 sprite 0xFF
        let mut rom = vec![0u8; 0x102];
        rom[0..8].copy_from_slice(&[0xA3, 0x00, 0x6A, 0x3E, 0x6B, 0x00, 0xDA, 0x01]);
        rom[0x100] = 0xFF;

        let mut vip = emu_with(&rom, 0);
        for _ in 0..4 {
            vip.tick();
        }
        // Lores pixel 62/63 -> hires columns 124/125.
        assert_eq!(vip.get_pixel(124, 0), 1);
        assert_eq!(vip.get_pixel(125, 0), 1);
        assert_eq!(vip.get_pixel(0, 0), 0, "VIP must clip, not wrap");
        assert_eq!(vip.get_pixel(1, 0), 0);

        let mut octo = emu_with(&rom, 1);
        for _ in 0..4 {
            octo.tick();
        }
        assert_eq!(octo.get_pixel(124, 0), 1);
        assert_eq!(octo.get_pixel(0, 0), 1, "Octo wraps by default");
        assert_eq!(octo.get_pixel(11, 0), 1);
    }

    #[test]
    fn hires_switch_clears_screen() {
        let mut rom = vec![0u8; 0x102];
        rom[0..10].copy_from_slice(&[0xA3, 0x00, 0x6A, 0x3E, 0x6B, 0x00, 0xDA, 0x01, 0x00, 0xFF]);
        rom[0x100] = 0xFF;

        let mut emu = emu_with(&rom, 1);
        for _ in 0..4 {
            emu.tick();
        }
        assert_eq!(emu.get_pixel(124, 0), 1);
        emu.tick(); // 00FF
        assert!(emu.is_hi_res());
        assert!(emu.screen[0].iter().all(|&p| !p));
        assert!(emu.screen[1].iter().all(|&p| !p));
    }

    #[test]
    fn xo_long_i_and_skip() {
        // v0 = 0; if v0 == 0 skip the 4-byte i := long 0x4000; v1 = 1
        let rom = [0x60, 0x00, 0x30, 0x00, 0xF0, 0x00, 0x40, 0x00, 0x61, 0x01];
        let mut emu = emu_with(&rom, 1);
        for _ in 0..3 {
            emu.tick();
        }
        assert_eq!(emu.get_reg(0x1), 1);
        assert_eq!(emu.get_pc(), 0x20A);
        assert_eq!(emu.get_i(), 0);

        // Without the skip, F000 loads the long address.
        let mut emu = emu_with(&rom[4..], 1);
        emu.tick();
        assert_eq!(emu.get_i(), 0x4000);
        assert_eq!(emu.get_pc(), 0x204);
    }

    #[test]
    fn xo_range_save_and_load() {
        let rom = [
            0x60, 0x11, // v0 = 0x11
            0x61, 0x22, // v1 = 0x22
            0xA4, 0x00, // i = 0x400
            0x50, 0x12, // save v0..v1
            0x60, 0x00, 0x61, 0x00, // clear v0, v1
            0x50, 0x13, // load v0..v1
        ];
        let mut emu = emu_with(&rom, 1);
        for _ in 0..7 {
            emu.tick();
        }
        assert_eq!(emu.get_ram(0x400), 0x11);
        assert_eq!(emu.get_ram(0x401), 0x22);
        assert_eq!(emu.get_reg(0), 0x11);
        assert_eq!(emu.get_reg(1), 0x22);
        assert_eq!(emu.get_i(), 0x400, "5XY2/5XY3 must not move i");
    }

    #[test]
    fn xo_range_save_reverses_registers() {
        let rom = [
            0x60, 0x11, 0x61, 0x22, 0xA4, 0x00, 0x51, 0x02, // save v1..v0
        ];
        let mut emu = emu_with(&rom, 1);
        for _ in 0..4 {
            emu.tick();
        }
        assert_eq!(emu.get_ram(0x400), 0x22);
        assert_eq!(emu.get_ram(0x401), 0x11);
    }

    #[test]
    fn scroll_up_moves_pixels() {
        let rom = [
            0xA3, 0x00, 0x6B, 0x02, 0xDA, 0xB1, // draw at y=2
            0x00, 0xD1, // scroll up 1
        ];
        let mut romv = rom.to_vec();
        romv.resize(0x102, 0);
        romv[0x100] = 0xFF;
        let mut emu = emu_with(&romv, 1);
        for _ in 0..3 {
            emu.tick();
        }
        assert_eq!(emu.get_pixel(0, 4), 1);
        emu.tick();
        assert_eq!(emu.get_pixel(0, 4), 0);
        assert_eq!(emu.get_pixel(0, 2), 1);
    }

    #[test]
    fn audio_pattern_defaults_and_can_be_replaced() {
        let emu = emu_with(&[0x60, 0x00], 1);
        assert_eq!(*emu.audio_pattern(), DEFAULT_AUDIO_PATTERN);
        assert_eq!(emu.pitch(), 64);

        let pattern = [
            0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0, 0x01, 0x23, 0x45, 0x67, 0x89, 0xAB,
            0xCD, 0xEF,
        ];
        let mut rom = vec![0u8; 0x210];
        rom[..8].copy_from_slice(&[
            0xA4, 0x00, // i = 0x400
            0xF0, 0x02, // audio
            0x60, 0x40, // v0 = 64
            0xF0, 0x3A, // pitch = v0
        ]);
        rom[0x200..0x210].copy_from_slice(&pattern);

        let mut emu = emu_with(&rom, 1);
        emu.tick();
        emu.tick();
        assert_eq!(*emu.audio_pattern(), pattern);
        assert_eq!(emu.pitch(), 64, "pitch is unchanged by F002");

        emu.tick();
        emu.tick();
        assert_eq!(emu.pitch(), 0x40);
    }
}
