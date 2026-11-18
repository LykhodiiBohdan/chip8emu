// Regression test for the inline XO-CHIP audio worklet in web/index.js.
//
// The worklet source lives in a template literal so that `wasm-pack` can keep
// rewriting web/.gitignore. `node --check web/index.js` cannot see syntax
// errors inside that string, so this script evaluates it with tiny stubs and
// checks the generated waveform.

import fs from "node:fs";

const SAMPLE_RATE = 48_000;
const indexSource = fs.readFileSync(new URL("../web/index.js", import.meta.url), "utf8");

const marker = "const XO_AUDIO_WORKLET = `";
const start = indexSource.indexOf(marker);
if (start === -1) {
    throw new Error("XO_AUDIO_WORKLET not found in web/index.js");
}
const end = indexSource.indexOf("`;", start + marker.length);
if (end === -1) {
    throw new Error("unterminated XO_AUDIO_WORKLET template literal");
}
const workletSource = indexSource.slice(start + marker.length, end);

let Processor = null;
globalThis.sampleRate = SAMPLE_RATE;
globalThis.AudioWorkletProcessor = class {
    constructor() {
        this.port = { onmessage: null };
    }
};
globalThis.registerProcessor = (_name, processor) => {
    Processor = processor;
};

new Function(workletSource)();
if (!Processor) {
    throw new Error("worklet did not call registerProcessor");
}

function render(pattern, pitch, seconds = 1.2) {
    const processor = new Processor();
    processor.port.onmessage({ data: { pattern, pitch, playing: true } });

    const total = Math.floor(SAMPLE_RATE * seconds);
    const samples = new Float32Array(total);
    const block = new Float32Array(128);

    for (let offset = 0; offset < total; offset += block.length) {
        block.fill(0);
        processor.process([], [[block]]);
        samples.set(block.subarray(0, Math.min(block.length, total - offset)), offset);
    }

    return samples;
}

const SETTLE_SECONDS = 0.2;

function peak(samples) {
    let max = 0;
    for (let i = Math.floor(SAMPLE_RATE * SETTLE_SECONDS); i < samples.length; i++) {
        max = Math.max(max, Math.abs(samples[i]));
    }
    return max;
}

function zeroCrossingsHz(samples) {
    const skip = Math.floor(SAMPLE_RATE * SETTLE_SECONDS);
    let crossings = 0;
    for (let i = skip + 1; i < samples.length; i++) {
        if (
            (samples[i - 1] < 0 && samples[i] >= 0) ||
            (samples[i - 1] >= 0 && samples[i] < 0)
        ) {
            crossings++;
        }
    }
    return (crossings * SAMPLE_RATE) / (2 * (samples.length - skip));
}

const silent = render(new Uint8Array(16), 64);
const defaultTone = render(new Uint8Array(16).fill(0x0f), 64);

// The example from the XO-CHIP specification: an A4 (439.06 Hz) waveform.
const a4Pattern = new Uint8Array(16);
a4Pattern.fill(0xff, 8);
const a4 = render(a4Pattern, 247);

const results = {
    silentPeak: peak(silent),
    defaultPeak: peak(defaultTone),
    defaultHz: zeroCrossingsHz(defaultTone),
    a4Hz: zeroCrossingsHz(a4),
};

console.log(JSON.stringify(results, null, 2));

const failures = [];
if (results.silentPeak > 0.01) failures.push("zero pattern must be silent");
if (results.defaultPeak < 0.05) failures.push("default pattern must be audible");
if (Math.abs(results.defaultHz - 500) > 25) failures.push("default pattern must be ~500 Hz");
if (Math.abs(results.a4Hz - 439) > 25) failures.push("spec A4 pattern must be ~439 Hz");

if (failures.length) {
    for (const failure of failures) console.error(`FAIL: ${failure}`);
    process.exit(1);
}

console.log("XO worklet checks passed");
