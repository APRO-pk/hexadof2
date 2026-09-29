// Generate the HexaDOF application icons.
//
// The icons are produced rather than checked in as binary blobs, so the mark is
// reproducible and reviewable: a dark graphite tile with an electric cyan hexagon
// and a launch axis. Run with `node scripts/generate-icons.mjs` from `src-tauri`.

import { deflateSync } from "node:zlib";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const outDir = join(here, "..", "icons");
mkdirSync(outDir, { recursive: true });

// Palette, matching the dark theme tokens in the frontend design system.
const BACKGROUND = [14, 17, 22, 255];
const PANEL = [22, 27, 34, 255];
const ACCENT = [56, 199, 255, 255];
const ACCENT_DIM = [30, 110, 145, 255];
const TEXT = [232, 238, 245, 255];

/** Draw the mark into an RGBA buffer of size x size. */
function draw(size) {
  const pixels = new Uint8Array(size * size * 4);
  const set = (x, y, colour) => {
    if (x < 0 || y < 0 || x >= size || y >= size) return;
    const i = (y * size + x) * 4;
    pixels[i] = colour[0];
    pixels[i + 1] = colour[1];
    pixels[i + 2] = colour[2];
    pixels[i + 3] = colour[3];
  };
  // Fill with a vertical gradient so the tile reads as a panel rather than a
  // flat square at large sizes.
  for (let y = 0; y < size; y += 1) {
    const t = y / Math.max(1, size - 1);
    const colour = [
      Math.round(BACKGROUND[0] + (PANEL[0] - BACKGROUND[0]) * t),
      Math.round(BACKGROUND[1] + (PANEL[1] - BACKGROUND[1]) * t),
      Math.round(BACKGROUND[2] + (PANEL[2] - BACKGROUND[2]) * t),
      255,
    ];
    for (let x = 0; x < size; x += 1) set(x, y, colour);
  }

  const cx = (size - 1) / 2;
  const cy = (size - 1) / 2;
  const radius = size * 0.36;
  const stroke = Math.max(1, size * 0.045);

  // A regular hexagon, which is the product mark and also the six degrees of
  // freedom the application is named for.
  const corner = (i) => {
    const angle = (Math.PI / 3) * i - Math.PI / 2;
    return [cx + radius * Math.cos(angle), cy + radius * Math.sin(angle)];
  };
  const corners = [0, 1, 2, 3, 4, 5].map(corner);

  const drawLine = (a, b, colour) => {
    const steps = Math.ceil(Math.max(Math.abs(b[0] - a[0]), Math.abs(b[1] - a[1])) * 2) + 1;
    for (let s = 0; s <= steps; s += 1) {
      const t = s / steps;
      const x = Math.round(a[0] + (b[0] - a[0]) * t);
      const y = Math.round(a[1] + (b[1] - a[1]) * t);
      const half = Math.floor(stroke / 2);
      for (let dx = -half; dx <= half; dx += 1) {
        for (let dy = -half; dy <= half; dy += 1) set(x + dx, y + dy, colour);
      }
    }
  };

  for (let i = 0; i < 6; i += 1) {
    drawLine(corners[i], corners[(i + 1) % 6], ACCENT);
  }

  // The launch axis: a vertical line through the centre with an arrowhead, so the
  // mark reads as flight dynamics rather than a generic hexagon.
  const top = [cx, cy - radius * 0.62];
  const bottom = [cx, cy + radius * 0.62];
  drawLine(top, bottom, TEXT);
  drawLine(top, [cx - radius * 0.26, cy - radius * 0.18], ACCENT_DIM);
  drawLine(top, [cx + radius * 0.26, cy - radius * 0.18], ACCENT_DIM);

  return pixels;
}

/** Encode an RGBA buffer as a PNG. */
function encodePng(pixels, size) {
  const raw = Buffer.alloc((size * 4 + 1) * size);
  for (let y = 0; y < size; y += 1) {
    raw[y * (size * 4 + 1)] = 0; // filter: none
    Buffer.from(pixels.buffer, y * size * 4, size * 4).copy(
      raw,
      y * (size * 4 + 1) + 1,
    );
  }
  const chunk = (type, data) => {
    const length = Buffer.alloc(4);
    length.writeUInt32BE(data.length, 0);
    const body = Buffer.concat([Buffer.from(type, "ascii"), data]);
    const crc = Buffer.alloc(4);
    crc.writeUInt32BE(crc32(body) >>> 0, 0);
    return Buffer.concat([length, body, crc]);
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // colour type: RGBA
  ihdr[10] = 0;
  ihdr[11] = 0;
  ihdr[12] = 0;
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

const CRC_TABLE = (() => {
  const table = new Uint32Array(256);
  for (let n = 0; n < 256; n += 1) {
    let c = n;
    for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c >>> 0;
  }
  return table;
})();

function crc32(buffer) {
  let c = 0xffffffff;
  for (const byte of buffer) c = CRC_TABLE[(c ^ byte) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

/** Encode an RGBA buffer as a single-image ICO holding a 32-bit BMP. */
function encodeIco(pixels, size) {
  const maskStride = Math.ceil(size / 32) * 4;
  const maskSize = maskStride * size;
  const header = Buffer.alloc(40);
  header.writeUInt32LE(40, 0);
  header.writeInt32LE(size, 4);
  // The DIB height covers the colour image plus the AND mask.
  header.writeInt32LE(size * 2, 8);
  header.writeUInt16LE(1, 12);
  header.writeUInt16LE(32, 14);
  header.writeUInt32LE(0, 16);
  header.writeUInt32LE(size * size * 4 + maskSize, 20);
  header.writeInt32LE(0, 24);
  header.writeInt32LE(0, 28);
  header.writeUInt32LE(0, 32);
  header.writeUInt32LE(0, 36);

  // BMP pixel rows are bottom-up and stored BGRA.
  const colour = Buffer.alloc(size * size * 4);
  for (let y = 0; y < size; y += 1) {
    const sourceRow = size - 1 - y;
    for (let x = 0; x < size; x += 1) {
      const s = (sourceRow * size + x) * 4;
      const d = (y * size + x) * 4;
      colour[d] = pixels[s + 2];
      colour[d + 1] = pixels[s + 1];
      colour[d + 2] = pixels[s];
      colour[d + 3] = pixels[s + 3];
    }
  }
  const mask = Buffer.alloc(maskSize);

  const directory = Buffer.alloc(6);
  directory.writeUInt16LE(0, 0);
  directory.writeUInt16LE(1, 2);
  directory.writeUInt16LE(1, 4);
  const entry = Buffer.alloc(16);
  entry[0] = size >= 256 ? 0 : size;
  entry[1] = size >= 256 ? 0 : size;
  entry[2] = 0;
  entry[3] = 0;
  entry.writeUInt16LE(1, 4);
  entry.writeUInt16LE(32, 6);
  const image = Buffer.concat([header, colour, mask]);
  entry.writeUInt32LE(image.length, 8);
  entry.writeUInt32LE(22, 12);
  return Buffer.concat([directory, entry, image]);
}

const sizes = [32, 64, 128, 256, 512];
const written = [];
for (const size of sizes) {
  const pixels = draw(size);
  writeFileSync(join(outDir, `${size}x${size}.png`), encodePng(pixels, size));
  written.push(`${size}x${size}.png`);
}

// Tauri's bundler expects these specific names.
const icon32 = draw(32);
writeFileSync(join(outDir, "icon.ico"), encodeIco(icon32, 32));
written.push("icon.ico");

const png128 = draw(128);
writeFileSync(join(outDir, "128x128.png"), encodePng(png128, 128));
const png256 = draw(256);
writeFileSync(join(outDir, "128x128@2x.png"), encodePng(png256, 256));
const png512 = draw(512);
writeFileSync(join(outDir, "icon.png"), encodePng(png512, 512));
written.push("128x128.png", "128x128@2x.png", "icon.png");

console.log(`Wrote ${written.length} icons to ${outDir}`);
for (const name of written) console.log(`  ${name}`);
