import { LosslessDecoder } from './static/decoder.js';

Deno.test("WASM Bounds Checking (Truncated FLAC)", async () => {
    const decoder = new LosslessDecoder();
    await decoder.init();

    const streamInfo = new Uint8Array(42);
    streamInfo.set([0x66, 0x4C, 0x61, 0x43], 0); // fLaC
    streamInfo[4] = 0x80;
    streamInfo[7] = 0x22;
    streamInfo[18] = 0x0B;
    streamInfo[19] = 0xB8;
    streamInfo[20] = 0x01 | 0x02; // 0x03
    streamInfo[21] = 0x70;

    let decoded = decoder.decode(streamInfo);
    if (decoded.length !== 0) throw new Error("Should not decode header");

    const truncatedFrame = new Uint8Array([0xFF, 0xF8, 0x7C, 0x00, 0x00, 0x11, 0x22]);
    decoded = decoder.decode(truncatedFrame);

    if (decoded.length !== 0) throw new Error("Should not decode truncated frame");

    console.log("WASM Bounds Checking Passed");
});

Deno.test("Normalization Verification", async () => {
    const decoder = new LosslessDecoder();
    await decoder.init();

    const streamInfo = new Uint8Array(42);
    streamInfo.set([0x66, 0x4C, 0x61, 0x43], 0); // fLaC
    streamInfo[4] = 0x80;
    streamInfo[7] = 0x22;
    streamInfo[18] = 0x0B;
    streamInfo[19] = 0xB8;
    streamInfo[20] = 0x03;
    streamInfo[21] = 0x70;

    decoder.decode(streamInfo);

    const frame = new Uint8Array([
        0xFF, 0xF8, // Sync
        0x7C, 0x16, // bs_sr, ch_bps
        0x00, // utf8
        0x00, 0x01, // block size
        0xBB, 0x80, // sample rate
        0x00, // CRC8
        // Subframe 0 (Verbatim)
        0x02,
        0x7F, 0xFF, 0xFF, // 0x7FFFFF (Max positive 24-bit)
        0x00, 0x00, 0x00, // 0
        // Subframe 1 (Verbatim)
        0x02,
        0x7F, 0xFF, 0xFF, // 0x7FFFFF
        0x00, 0x00, 0x00, // 0
        // CRC16
        0x00, 0x00
    ]);

    const decoded = decoder.decode(frame);
    const firstSample = decoded[0];

    const diff = Math.abs(firstSample - 0.99999988);
    if (diff > 0.0000001) {
        throw new Error(`Normalization failed. Expected ~0.99999988, got ${firstSample}`);
    }

    console.log("Normalization Verification Passed");
});
