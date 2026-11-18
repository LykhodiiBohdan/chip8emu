//! Headless smoke test against the Timendus CHIP-8 test suite.
//!
//! Point `CHIP8_TEST_SUITE_DIR` at the `bin/` directory of
//! <https://github.com/Timendus/chip8-test-suite>. Each ROM is executed on its
//! supported profiles for a bounded number of ticks; the test only fails if
//! the interpreter halts with an error. The suite's visual pass/fail screens
//! are intentionally not interpreted here.

use chip8_core::*;
use std::env;
use std::fs;
use std::path::Path;

/// Enough for the test ROMs to finish their self-tests on any profile.
const MAX_TICKS: usize = 2_000_000;

const OCTO: u8 = 1;
const VIP: u8 = 0;
const SCHIP: u8 = 2;

fn main() {
    let dir = env::var("CHIP8_TEST_SUITE_DIR").unwrap_or_else(|_| {
        eprintln!("CHIP8_TEST_SUITE_DIR must point at the chip8-test-suite bin/ directory");
        std::process::exit(2);
    });

    let cases: &[(&str, &[u8])] = &[
        ("1-chip8-logo.ch8", &[VIP, OCTO, SCHIP]),
        ("2-ibm-logo.ch8", &[VIP, OCTO, SCHIP]),
        ("3-corax+.ch8", &[VIP, OCTO, SCHIP]),
        ("4-flags.ch8", &[VIP, OCTO, SCHIP]),
        ("5-quirks.ch8", &[VIP, OCTO, SCHIP]),
        // 6-keypad.ch8 waits for interactive input and is skipped.
        ("7-beep.ch8", &[OCTO]),
        ("8-scrolling.ch8", &[SCHIP, OCTO]),
    ];

    let mode_names = ["VIP", "OCTO", "SCHIP"];
    let mut failures = 0;

    for (name, modes) in cases {
        let path = Path::new(&dir).join(name);
        let data = match fs::read(&path) {
            Ok(data) => data,
            Err(error) => {
                eprintln!("FAIL {name}: cannot read {}: {error}", path.display());
                failures += 1;
                continue;
            }
        };

        for &mode in *modes {
            let mut emu = Emu::new();
            emu.set_mode(mode);
            if !emu.load(&data) {
                eprintln!("FAIL {name}: ROM was rejected by load()");
                failures += 1;
                continue;
            }

            let mut ticks = 0;
            while ticks < MAX_TICKS && !emu.is_halted() {
                emu.tick();
                ticks += 1;
            }

            match emu.halt_reason() {
                None | Some(HaltReason::Exit) => {
                    println!(
                        "ok   {name:<20} {:<5} {ticks:>8} ticks",
                        mode_names[mode as usize]
                    );
                }
                Some(reason) => {
                    eprintln!(
                        "FAIL {name:<20} {:<5} {reason:?} at pc {:04X} (last op {:04X}, {ticks} ticks)",
                        mode_names[mode as usize],
                        emu.get_pc(),
                        emu.last_opcode()
                    );
                    failures += 1;
                }
            }
        }
    }

    if failures > 0 {
        eprintln!("{failures} test-suite run(s) failed");
        std::process::exit(1);
    }

    println!("chip8-test-suite smoke test passed");
}
